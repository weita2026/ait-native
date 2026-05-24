from __future__ import annotations

import re
from pathlib import Path
from typing import Any, Callable, Mapping

from ..server_paths import ServerContext


def _legacy_read_models_module():
    from .. import read_models as legacy_read_models

    return legacy_read_models


AuthorityMarkdownLoader = Callable[[str], str | None]


rm = _legacy_read_models_module()


AUTHORITY_LAYER2_PATHS: tuple[str, ...] = (
    "docs/product_plan.md",
    "docs/market_strategy.md",
    "docs/engineering_plan.md",
    "docs/financial_plan.md",
    "docs/legal_plan.md",
)
AUTHORITY_CENTER_NODE_PATHS: tuple[str, ...] = ("docs/milestone.md",)
AUTHORITY_MAP_TABLES: tuple[str, ...] = ("authority_maps", "authority_nodes")
AUTHORITY_PARENT_OVERRIDES: dict[str, str] = {
    "AGENTS.md": "docs/engineering_plan.md",
    "ait.md": "docs/engineering_plan.md",
    "docs/ait_solo_local.md": "docs/engineering_plan.md",
    "docs/ait_solo_remote.md": "docs/engineering_plan.md",
    "docs/ait_team_remote.md": "docs/engineering_plan.md",
    "docs/ait.md": "docs/engineering_plan.md",
}


def _authority_node_layer(node_kind: str) -> int:
    kind = (node_kind or "").strip().lower()
    if kind == "layer1":
        return 1
    if kind == "milestone":
        return 3
    if kind == "layer2":
        return 2
    if kind == "layer3":
        return 3
    return 3


def _authority_node_sort_key(node: dict[str, Any]) -> tuple[int, str, str]:
    return (
        rm._safe_int(node.get("sort_index")),
        str(node.get("short_title") or node.get("title") or "").lower(),
        str(node.get("path") or ""),
    )


def _authority_doc_missing_row(path: str, *, layer: int, title: str | None) -> dict[str, Any]:
    fallback_title = str(title or "").strip() or str(path).strip() or "Untitled"
    return {
        "doc_id": path,
        "path": path,
        "filename": Path(path).name,
        "title": fallback_title,
        "short_title": _document_short_title(fallback_title, path),
        "layer": layer,
        "status": "current",
        "scope": "",
        "authority": "",
        "markdown": "",
        "body_markdown": "",
        "related_paths": [],
        "authority_link_paths": [],
    }


def _authority_db_row_to_doc(
    load_markdown: "AuthorityMarkdownLoader",
    row: Mapping[str, Any],
    *,
    layer: int,
) -> dict[str, Any]:
    path = str(row.get("document_path") or "").strip()
    if not path:
        path = str(row.get("path") or "").strip()
    if not path:
        path = str(row.get("slug") or "unknown").strip()
    row_title = str(row.get("title") or "").strip()
    doc = _read_authority_doc(load_markdown, path, layer=layer)
    if doc is None:
        doc = _authority_doc_missing_row(path, layer=layer, title=row_title or None)
    if row_title:
        doc["title"] = row_title
        doc["short_title"] = _document_short_title(row_title, path)
    doc["doc_id"] = path
    doc["authority_node_id"] = str(row.get("authority_node_id") or "")
    doc["authority_map_id"] = str(row.get("authority_map_id") or "")
    node_kind = str(row.get("node_kind") or f"layer{layer}")
    if not node_kind:
        node_kind = f"layer{layer}"
    doc["node_kind"] = node_kind
    doc["parent_node_id"] = str(row.get("parent_node_id")) if row.get("parent_node_id") is not None else None
    doc["sort_index"] = rm._safe_int(row.get("sort_index"))
    doc["slug"] = str(row.get("slug") or "")
    doc["connection_mode"] = str(row.get("connection_mode") or "")
    return doc


def _header_metadata(markdown: str) -> dict[str, str]:
    metadata: dict[str, str] = {}
    for line in (markdown or "").splitlines():
        match = re.match(r"^(Authority|Status|Scope):\s*(.+?)\s*$", line.strip())
        if not match:
            continue
        key, value = match.groups()
        metadata[key.lower()] = value.strip()
    return metadata


def _body_markdown(markdown: str) -> str:
    lines = (markdown or "").splitlines()
    index = 0
    if lines and lines[0].startswith("# "):
        index += 1
    while index < len(lines) and not lines[index].strip():
        index += 1
    return "\n".join(lines[index:]).strip()


def _document_short_title(title: str, path: str) -> str:
    stem = Path(path).stem.replace("_", " ").strip()
    return title or stem or path


def _command_layer_paths(load_markdown: AuthorityMarkdownLoader, candidate_paths: list[str] | None = None) -> list[str]:
    seed_paths = [
        "AGENTS.md",
        "ait.md",
        "docs/ait_solo_local.md",
        "docs/ait_solo_remote.md",
        "docs/ait_team_remote.md",
        "docs/ait.md",
        "docs/milestone.md",
        "docs/markdown_document_map.md",
        "docs/markdown_document_organization.md",
        "docs/ait_native_quickstart.md",
        "docs/ait_native_runtime_operations.md",
    ]
    excluded = {"docs/plan.md", *AUTHORITY_LAYER2_PATHS}
    queue = [path for path in rm._ordered_unique([*seed_paths, *(candidate_paths or [])]) if load_markdown(path) is not None]
    seen: set[str] = set()
    discovered: list[str] = []
    while queue:
        path = queue.pop(0)
        if path in seen:
            continue
        seen.add(path)
        markdown = load_markdown(path)
        if markdown is None:
            continue
        if path not in excluded:
            discovered.append(path)
        for target in rm._markdown_link_targets(markdown):
            normalized = rm._normalize_markdown_target(path, target)
            if normalized is None or normalized in seen or normalized in excluded:
                continue
            if load_markdown(normalized) is not None:
                queue.append(normalized)
    return rm._ordered_unique(discovered)


def _read_authority_doc(load_markdown: AuthorityMarkdownLoader, path: str, *, layer: int) -> dict[str, Any] | None:
    markdown = load_markdown(path)
    if markdown is None:
        return None
    title = rm._document_title(markdown.splitlines(), Path(path).stem.replace("_", " "))
    metadata = _header_metadata(markdown)
    related_paths = rm._ordered_unique(
        normalized
        for target in rm._markdown_link_targets(markdown)
        if (normalized := rm._normalize_markdown_target(path, target)) is not None
    )
    authority_link_paths = rm._ordered_unique(
        normalized
        for target in rm._markdown_link_targets(metadata.get("authority", ""))
        if (normalized := rm._normalize_markdown_target(path, target)) is not None
    )
    return {
        "doc_id": path,
        "path": path,
        "filename": Path(path).name,
        "title": title,
        "short_title": _document_short_title(title, path),
        "layer": layer,
        "status": metadata.get("status", "current"),
        "scope": metadata.get("scope", ""),
        "authority": metadata.get("authority", ""),
        "markdown": markdown,
        "body_markdown": _body_markdown(markdown),
        "related_paths": related_paths,
        "authority_link_paths": authority_link_paths,
    }


def _authority_parent_path(doc: dict[str, Any], legal_paths: set[str]) -> str:
    path = str(doc.get("path") or "")
    override = AUTHORITY_PARENT_OVERRIDES.get(path)
    if override is not None:
        return override
    for related_path in doc.get("authority_link_paths", []):
        if related_path in legal_paths:
            return related_path
    lowered = path.lower()
    if any(token in lowered for token in ("financial", "economics", "runway", "cost")):
        return "docs/financial_plan.md"
    if any(token in lowered for token in ("market", "benchmark", "launch")):
        return "docs/market_strategy.md"
    if any(token in lowered for token in ("legal", "license", "privacy", "security", "attestation", "policy")):
        return "docs/legal_plan.md"
    if any(token in lowered for token in ("product", "segmentation", "ux_principles", "roadmap")):
        return "docs/product_plan.md"
    return "docs/engineering_plan.md"


def _authority_linked_doc_layer(path: str, legal_paths: set[str]) -> int:
    if path == "docs/plan.md":
        return 1
    if path in legal_paths:
        return 2
    return 3


def _authority_seed_doc_title(doc: dict[str, Any]) -> str:
    return (
        str(doc.get("short_title") or "").strip()
        or str(doc.get("title") or "").strip()
        or str(doc.get("filename") or "").strip()
        or "Untitled"
    )


def _authority_seed_sort_layer2_docs(docs: list[dict[str, Any]]) -> list[dict[str, Any]]:
    if any(rm._safe_int(doc.get("sort_index")) > 0 for doc in docs):
        return sorted(
            docs,
            key=lambda doc: (
                rm._safe_int(doc.get("sort_index")) or 10_000,
                str(doc.get("short_title") or doc.get("title") or "").lower(),
                str(doc.get("path") or ""),
            ),
        )
    return sorted(
        docs,
        key=lambda doc: (
            -len(doc.get("children", [])),
            str(doc.get("short_title") or doc.get("title") or "").lower(),
            str(doc.get("path") or ""),
        ),
    )


def _authority_seed_sort_layer3_docs(docs: list[dict[str, Any]]) -> list[dict[str, Any]]:
    if any(rm._safe_int(doc.get("sort_index")) > 0 for doc in docs):
        return sorted(
            docs,
            key=lambda doc: (
                rm._safe_int(doc.get("sort_index")) or 10_000,
                str(doc.get("filename") or doc.get("short_title") or doc.get("title") or "").lower(),
                str(doc.get("path") or ""),
            ),
        )
    return list(docs)


def _authority_nodes_from_model(model: dict[str, Any]) -> list[dict[str, Any]]:
    nodes: list[dict[str, Any]] = []
    layer1 = model.get("layer1")
    if isinstance(layer1, dict) and str(layer1.get("path") or "").strip():
        layer1_path = str(layer1["path"]).strip()
        nodes.append(
            {
                "node_kind": "layer1",
                "document_path": layer1_path,
                "title": _authority_seed_doc_title(layer1),
                "sort_index": 1,
                "connection_mode": str(layer1.get("connection_mode") or "connected"),
            }
        )
    else:
        raise ValueError("Authority map is missing docs/plan.md")

    for index, doc in enumerate(model.get("center_nodes") or [], start=2):
        if not isinstance(doc, dict):
            continue
        path = str(doc.get("path") or "").strip()
        if not path:
            continue
        nodes.append(
            {
                "node_kind": str(doc.get("node_kind") or "milestone"),
                "document_path": path,
                "title": _authority_seed_doc_title(doc),
                "sort_index": rm._safe_int(doc.get("sort_index")) or index,
                "connection_mode": str(doc.get("connection_mode") or "detached"),
            }
        )

    for layer2_index, doc in enumerate(_authority_seed_sort_layer2_docs(list(model.get("layer2") or [])), start=1):
        if not isinstance(doc, dict):
            continue
        path = str(doc.get("path") or "").strip()
        if not path:
            continue
        nodes.append(
            {
                "node_kind": "layer2",
                "document_path": path,
                "parent_document_path": layer1_path,
                "title": _authority_seed_doc_title(doc),
                "sort_index": rm._safe_int(doc.get("sort_index")) or layer2_index,
                "connection_mode": str(doc.get("connection_mode") or "connected"),
            }
        )
        for layer3_index, child in enumerate(_authority_seed_sort_layer3_docs(list(doc.get("children") or [])), start=1):
            if not isinstance(child, dict):
                continue
            child_path = str(child.get("path") or "").strip()
            if not child_path:
                continue
            nodes.append(
                {
                    "node_kind": "layer3",
                    "document_path": child_path,
                    "parent_document_path": path,
                    "title": _authority_seed_doc_title(child),
                    "sort_index": rm._safe_int(child.get("sort_index")) or layer3_index,
                    "connection_mode": str(child.get("connection_mode") or "connected"),
                }
            )
    return nodes


def _seed_authority_graph_from_markdown_model(
    ctx: ServerContext,
    repo_name: str,
    model: dict[str, Any],
    load_markdown: AuthorityMarkdownLoader,
) -> dict[str, Any] | None:
    try:
        if rm.list_authority_graph(ctx, repo_name) is not None:
            return None
        rm.ensure_repository(ctx, repo_name, "main")
        rm.replace_authority_graph(
            ctx,
            repo_name,
            nodes=_authority_nodes_from_model(model),
            actor_identity="system",
            actor_type="system_worker",
        )
    except Exception:
        return None
    return _authority_map_from_db(ctx, repo_name, load_markdown)


def _authority_map_from_markdown_source(load_markdown: AuthorityMarkdownLoader, candidate_paths: list[str] | None = None) -> dict[str, Any]:
    layer1 = _read_authority_doc(load_markdown, "docs/plan.md", layer=1) or _authority_doc_missing_row(
        "docs/plan.md",
        layer=1,
        title="Plan",
    )
    layer2_docs = [
        doc
        for path in AUTHORITY_LAYER2_PATHS
        if (doc := _read_authority_doc(load_markdown, path, layer=2)) is not None
    ]
    center_nodes = [
        doc
        for path in AUTHORITY_CENTER_NODE_PATHS
        if (doc := _read_authority_doc(load_markdown, path, layer=3)) is not None
    ]
    if not center_nodes:
        center_nodes = [_authority_doc_missing_row("docs/milestone.md", layer=3, title="Milestone Index")]
    for doc in center_nodes:
        doc["node_role"] = "milestone" if doc["path"] == "docs/milestone.md" else "center"
        doc["display_parent_path"] = "docs/engineering_plan.md"
    command_paths = _command_layer_paths(load_markdown, candidate_paths)
    legal_paths = {doc["path"] for doc in layer2_docs}
    center_paths = {doc["path"] for doc in center_nodes}
    layer3_docs = [
        doc
        for path in command_paths
        if path not in legal_paths and path not in center_paths and path != "docs/plan.md"
        if (doc := _read_authority_doc(load_markdown, path, layer=3)) is not None
    ]
    groups: dict[str, list[dict[str, Any]]] = {path: [] for path in legal_paths}
    for doc in layer3_docs:
        parent_path = _authority_parent_path(doc, legal_paths)
        doc["display_parent_path"] = parent_path
        groups.setdefault(parent_path, []).append(doc)
    documents_by_path: dict[str, dict[str, Any]] = {}
    if layer1 is not None:
        documents_by_path[layer1["path"]] = layer1
    for doc in center_nodes:
        documents_by_path[doc["path"]] = doc
    for doc in layer2_docs:
        children = sorted(groups.get(doc["path"], []), key=lambda item: (item["short_title"].lower(), item["path"]))
        doc["children"] = children
        documents_by_path[doc["path"]] = doc
        for child in children:
            documents_by_path[child["path"]] = child

    linked_documents: list[dict[str, Any]] = []
    linked_queue = rm._ordered_unique(
        [
            related_path
            for doc in documents_by_path.values()
            for related_path in doc.get("related_paths", [])
            if related_path not in documents_by_path
        ]
    )
    linked_index = 0
    while linked_index < len(linked_queue):
        path = linked_queue[linked_index]
        linked_index += 1
        if path in documents_by_path:
            continue
        linked_doc = _read_authority_doc(
            load_markdown,
            path,
            layer=_authority_linked_doc_layer(path, legal_paths),
        )
        if linked_doc is None:
            continue
        documents_by_path[path] = linked_doc
        linked_documents.append(linked_doc)
        for related_path in linked_doc.get("related_paths", []):
            if related_path not in documents_by_path and related_path not in linked_queue:
                linked_queue.append(related_path)

    for doc in documents_by_path.values():
        doc["related_documents"] = [
            {
                "path": related_path,
                "title": documents_by_path[related_path]["short_title"],
                "layer": documents_by_path[related_path]["layer"],
            }
            for related_path in doc.get("related_paths", [])
            if related_path in documents_by_path and related_path != doc["path"]
        ]
    relationship_count = len(layer2_docs)
    relationship_count += sum(len(doc.get("children", [])) for doc in layer2_docs)
    relationship_count += sum(len(doc.get("related_documents", [])) for doc in documents_by_path.values())
    return {
        "layer1": layer1,
        "center_nodes": center_nodes,
        "layer2": layer2_docs,
        "linked_documents": linked_documents,
        "summary": {
            "center_node_count": len(center_nodes),
            "layer2_count": len(layer2_docs),
            "layer3_count": len(layer3_docs) + len(center_nodes),
            "relationship_count": relationship_count,
        },
    }


def _authority_map_from_db(
    _ctx: ServerContext,
    repo_name: str,
    load_markdown: AuthorityMarkdownLoader,
) -> dict[str, Any] | None:
    try:
        with rm.connect(_ctx) as conn:
            if not all(rm._table_exists(conn, table_name) for table_name in AUTHORITY_MAP_TABLES):
                return None
            if rm._table_has_column(conn, "authority_maps", "repo_id"):
                scope_predicate, scope_params = rm._repo_scope_filter(_ctx, repo_name)
            else:
                scope_predicate = "repo_name = ?"
                scope_params = (repo_name,)
            map_row = conn.execute(
                "select authority_map_id, root_document_path, milestone_document_path from authority_maps where "
                f"{scope_predicate} limit 1",
                scope_params,
            ).fetchone()
            if map_row is None and rm._table_has_column(conn, "authority_maps", "repo_id"):
                map_row = conn.execute(
                    "select authority_map_id, root_document_path, milestone_document_path from authority_maps where repo_name = ? limit 1",
                    (repo_name,),
                ).fetchone()
            if map_row is None:
                return None
            authority_map_id = map_row.get("authority_map_id")
            if not authority_map_id:
                return None
            node_rows = conn.execute(
                "select * from authority_nodes where authority_map_id = ?",
                (authority_map_id,),
            ).fetchall()
    except Exception:
        return None

    root_document_path = str(map_row.get("root_document_path") or "docs/plan.md")
    milestone_document_path = str(map_row.get("milestone_document_path") or "docs/milestone.md")
    all_nodes: dict[str, dict[str, Any]] = {}
    nodes_by_id: dict[str, dict[str, Any]] = {}

    def _add_node(row: Mapping[str, Any], *, force_path: str | None = None, force_node_kind: str | None = None) -> None:
        kind = str(force_node_kind or row.get("node_kind") or "").strip()
        row_path = force_path or str(row.get("document_path") or "").strip()
        if not row_path:
            return
        synthetic_row = dict(row)
        synthetic_row["document_path"] = row_path
        if force_node_kind is not None:
            synthetic_row["node_kind"] = force_node_kind
        node_doc = _authority_db_row_to_doc(
            load_markdown,
            synthetic_row,
            layer=_authority_node_layer(kind),
        )
        node_id = str(synthetic_row.get("authority_node_id") or f"db:{row_path}").strip()
        if node_id:
            node_doc["authority_node_id"] = node_id
        if not row.get("authority_node_id"):
            node_doc["authority_node_id"] = node_id
        node_path = node_doc["path"]
        all_nodes[node_path] = node_doc
        nodes_by_id[node_id] = node_doc

    for row in node_rows:
        _add_node(row)

    if root_document_path not in all_nodes:
        _add_node(
            {"document_path": root_document_path, "authority_map_id": str(map_row.get("authority_map_id") or ""), "title": "Plan"},
            force_node_kind="layer1",
            force_path=root_document_path,
        )

    if milestone_document_path not in all_nodes:
        _add_node(
            {"document_path": milestone_document_path, "authority_map_id": str(map_row.get("authority_map_id") or ""), "title": "Milestone"},
            force_node_kind="milestone",
            force_path=milestone_document_path,
        )

    center_nodes = [doc for doc in all_nodes.values() if str(doc.get("node_kind")) == "milestone"]
    layer1 = (
        all_nodes.get(root_document_path)
        or next((doc for doc in all_nodes.values() if str(doc.get("node_kind")) == "layer1"), None)
    )
    layer2_docs = [doc for doc in all_nodes.values() if str(doc.get("node_kind")) == "layer2"]
    legal_paths = {doc["path"] for doc in layer2_docs}

    for doc in center_nodes:
        doc["node_role"] = "milestone" if doc["path"] == "docs/milestone.md" else "center"
        doc["display_parent_path"] = "docs/engineering_plan.md"

    children_by_parent: dict[str, list[dict[str, Any]]] = {path: [] for path in legal_paths}
    detached_layer3: list[dict[str, Any]] = []
    for node in list(all_nodes.values()):
        if str(node.get("node_kind")) != "layer3":
            continue
        parent_node_id = str(node.get("parent_node_id") or "").strip()
        parent = nodes_by_id.get(parent_node_id)
        parent_path = str(parent.get("path") or "").strip() if parent else ""
        if parent_path:
            node["display_parent_path"] = parent_path
            children_by_parent.setdefault(parent_path, []).append(node)
            continue
        parent_path = _authority_parent_path(node, legal_paths)
        node["display_parent_path"] = parent_path
        if parent_path in legal_paths:
            children_by_parent.setdefault(parent_path, []).append(node)
            continue
        detached_layer3.append(node)

    for doc in layer2_docs:
        doc["children"] = sorted(children_by_parent.get(doc["path"], []), key=_authority_node_sort_key)

    documents_by_path: dict[str, dict[str, Any]] = {}
    if layer1 is not None:
        documents_by_path[layer1["path"]] = layer1
    for doc in center_nodes:
        documents_by_path[doc["path"]] = doc
    for doc in layer2_docs:
        documents_by_path[doc["path"]] = doc
        for child in doc.get("children", []):
            documents_by_path[child["path"]] = child

    linked_documents: list[dict[str, Any]] = []
    for doc in detached_layer3:
        if doc["path"] in documents_by_path:
            continue
        linked_documents.append(doc)

    for doc in linked_documents:
        documents_by_path[doc["path"]] = doc

    linked_documents = sorted(linked_documents, key=_authority_node_sort_key)

    linked_queue = rm._ordered_unique(
        [
            related_path
            for doc in documents_by_path.values()
            for related_path in doc.get("related_paths", [])
            if related_path not in documents_by_path
        ]
    )
    linked_index = 0
    while linked_index < len(linked_queue):
        path = linked_queue[linked_index]
        linked_index += 1
        if path in documents_by_path:
            continue
        linked_doc = _read_authority_doc(load_markdown, path, layer=_authority_linked_doc_layer(path, legal_paths))
        if linked_doc is None:
            continue
        documents_by_path[path] = linked_doc
        linked_documents.append(linked_doc)
        for related_path in linked_doc.get("related_paths", []):
            if related_path not in documents_by_path and related_path not in linked_queue:
                linked_queue.append(related_path)

    for doc in documents_by_path.values():
        doc["related_documents"] = [
            {
                "path": related_path,
                "title": documents_by_path[related_path]["short_title"],
                "layer": documents_by_path[related_path]["layer"],
            }
            for related_path in doc.get("related_paths", [])
            if related_path in documents_by_path and related_path != doc["path"]
        ]

    layer2_docs = sorted(layer2_docs, key=_authority_node_sort_key)

    relationship_count = len(layer2_docs)
    relationship_count += sum(len(doc.get("children", [])) for doc in layer2_docs)
    relationship_count += sum(len(doc.get("related_documents", [])) for doc in documents_by_path.values())
    layer3_count = len([doc for doc in all_nodes.values() if str(doc.get("node_kind")) == "layer3"])

    return {
        "layer1": layer1,
        "center_nodes": center_nodes,
        "layer2": layer2_docs,
        "linked_documents": linked_documents,
        "summary": {
            "center_node_count": len(center_nodes),
            "layer2_count": len(layer2_docs),
            "layer3_count": layer3_count + len(center_nodes),
            "relationship_count": relationship_count,
        },
    }


def authority_map(_ctx: ServerContext, repo_name: str | None = None) -> dict[str, Any]:
    repo_root = rm._repo_root()
    local_repo_name = rm._repo_display_name(repo_root)
    resolved_repo_name = str(repo_name or "").strip() or local_repo_name
    local_mode = resolved_repo_name == local_repo_name
    load_markdown = rm._local_markdown_loader(repo_root) if local_mode else rm._snapshot_markdown_loader(_ctx, resolved_repo_name)
    candidate_paths = rm._local_markdown_paths(repo_root) if local_mode else rm._snapshot_markdown_paths(_ctx, resolved_repo_name)
    db_model = _authority_map_from_db(_ctx, resolved_repo_name, load_markdown)
    if db_model is not None:
        db_model["repo_name"] = resolved_repo_name
        db_model["interactive"] = resolved_repo_name == local_repo_name
        return db_model
    markdown_model = _authority_map_from_markdown_source(load_markdown, candidate_paths)
    seeded_model = _seed_authority_graph_from_markdown_model(_ctx, resolved_repo_name, markdown_model, load_markdown)
    if seeded_model is not None:
        seeded_model["repo_name"] = resolved_repo_name
        seeded_model["interactive"] = resolved_repo_name == local_repo_name
        return seeded_model
    markdown_model["repo_name"] = resolved_repo_name
    markdown_model["interactive"] = resolved_repo_name == local_repo_name
    return markdown_model
