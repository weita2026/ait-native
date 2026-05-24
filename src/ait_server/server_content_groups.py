from __future__ import annotations

import hashlib
import uuid
from typing import Any

from ait_protocol.common import utc_now

from .server_paths import ServerContext

DEFAULT_REPOSITORY_GROUP_TITLE = "Main group"
DEFAULT_REPOSITORY_GROUP_SYSTEM_SLUG = "main-group"


def _server_content_module():
    from . import server_content as _server_content

    return _server_content


def _repository_id_scope_predicate(alias: str | None = None) -> str:
    prefix = f"{alias}." if alias is not None else ""
    return f"({prefix}repo_id = ? or ({prefix}repo_id is null and {prefix}repo_name = ?))"


def _repository_group_row_out(row: dict[str, Any] | Any) -> dict[str, Any]:
    out = dict(row)
    out["title"] = str(out.get("title") or "").strip() or DEFAULT_REPOSITORY_GROUP_TITLE
    out["sort_index"] = int(out.get("sort_index") or 0)
    out["system_slug"] = str(out.get("system_slug") or "").strip() or None
    return out


def _new_repository_group_id(seed: str | None = None) -> str:
    digest = hashlib.sha256(f"{seed or 'group'}|{utc_now()}|{uuid.uuid4().hex}".encode("utf-8")).hexdigest()[:16].upper()
    return f"RPG-{digest}"


def _list_repository_group_rows(conn) -> list[dict[str, Any]]:
    return [
        _repository_group_row_out(row)
        for row in conn.execute(
            "select * from repository_groups order by sort_index asc, created_at asc, group_id asc"
        ).fetchall()
    ]


def _normalize_repository_group_order(conn) -> list[dict[str, Any]]:
    now = utc_now()
    rows = _list_repository_group_rows(conn)
    for index, row in enumerate(rows, start=1):
        if int(row.get("sort_index") or 0) == index:
            continue
        conn.execute(
            "update repository_groups set sort_index = ?, updated_at = ? where group_id = ?",
            (index, now, row["group_id"]),
        )
    return _list_repository_group_rows(conn)


def _normalize_repository_group_memberships(conn, *, group_id: str | None = None) -> None:
    now = utc_now()
    params: tuple[Any, ...]
    sql = (
        "select repo_name, repo_id, group_id, sort_index from repository_group_memberships "
        "where group_id = ? order by sort_index asc, coalesce(repo_id, repo_name) asc"
    )
    if group_id is None:
        rows = [
            dict(row)
            for row in conn.execute(
                "select repo_name, repo_id, group_id, sort_index from repository_group_memberships "
                "order by group_id asc, sort_index asc, coalesce(repo_id, repo_name) asc"
            ).fetchall()
        ]
        grouped: dict[str, list[dict[str, Any]]] = {}
        for row in rows:
            grouped.setdefault(str(row["group_id"]), []).append(row)
        for candidate_group_id, entries in grouped.items():
            for index, entry in enumerate(entries, start=1):
                if int(entry.get("sort_index") or 0) == index:
                    continue
                repo_id = entry.get("repo_id")
                repo_name = str(entry["repo_name"])
                conn.execute(
                    "update repository_group_memberships set sort_index = ?, updated_at = ? where "
                    + _repository_id_scope_predicate(),
                    (index, now, repo_id, str(entry["repo_name"])),
                )
        return

    rows = [dict(row) for row in conn.execute(sql, (group_id,)).fetchall()]
    for index, row in enumerate(rows, start=1):
        if int(row.get("sort_index") or 0) == index:
            continue
        repo_id = row.get("repo_id")
        repo_name = str(row["repo_name"])
        conn.execute(
            "update repository_group_memberships set sort_index = ?, updated_at = ? where "
            + _repository_id_scope_predicate(),
            (index, now, repo_id, repo_name),
        )


def _ensure_default_repository_group(conn) -> dict[str, Any]:
    row = conn.execute(
        "select * from repository_groups where system_slug = ?",
        (DEFAULT_REPOSITORY_GROUP_SYSTEM_SLUG,),
    ).fetchone()
    if row is None:
        now = utc_now()
        group_id = _new_repository_group_id(DEFAULT_REPOSITORY_GROUP_SYSTEM_SLUG)
        conn.execute(
            """
            insert into repository_groups(group_id, title, sort_index, system_slug, created_at, updated_at)
            values (?, ?, 1, ?, ?, ?)
            """,
            (group_id, DEFAULT_REPOSITORY_GROUP_TITLE, DEFAULT_REPOSITORY_GROUP_SYSTEM_SLUG, now, now),
        )
        row = conn.execute("select * from repository_groups where group_id = ?", (group_id,)).fetchone()
    elif str(dict(row).get("title") or "").strip() != DEFAULT_REPOSITORY_GROUP_TITLE:
        now = utc_now()
        conn.execute(
            "update repository_groups set title = ?, updated_at = ? where group_id = ?",
            (DEFAULT_REPOSITORY_GROUP_TITLE, now, row["group_id"]),
        )
        row = conn.execute("select * from repository_groups where group_id = ?", (row["group_id"],)).fetchone()
    assert row is not None
    _normalize_repository_group_order(conn)
    return _repository_group_row_out(row)


def _sync_repository_group_memberships(conn) -> None:
    default_group = _ensure_default_repository_group(conn)
    current_rows = [
        dict(row)
        for row in conn.execute(
            """
            select repo_name, repo_id, group_id, sort_index
            from repository_group_memberships
            order by group_id asc, sort_index asc, coalesce(repo_id, repo_name) asc
            """
        ).fetchall()
    ]
    memberships = {str(row.get("repo_id") or row["repo_name"]): row for row in current_rows}
    per_group_counts: dict[str, int] = {}
    for row in current_rows:
        group_id = str(row["group_id"])
        per_group_counts[group_id] = max(per_group_counts.get(group_id, 0), int(row.get("sort_index") or 0))
    repos = [dict(row) for row in conn.execute("select repo_name, repo_id from repositories order by repo_name asc").fetchall()]
    if not repos:
        return
    now = utc_now()
    for row in repos:
        repo_name = str(row["repo_name"])
        repo_id = str(row.get("repo_id") or "").strip() or None
        membership_key = repo_id or repo_name
        if membership_key in memberships:
            continue
        sort_index = per_group_counts.get(default_group["group_id"], 0) + 1
        per_group_counts[default_group["group_id"]] = sort_index
        conn.execute(
            """
            insert into repository_group_memberships(repo_name, repo_id, group_id, sort_index, created_at, updated_at)
            values (?, ?, ?, ?, ?, ?)
            """,
            (repo_name, repo_id, default_group["group_id"], sort_index, now, now),
        )
    _normalize_repository_group_memberships(conn)



def list_repository_groups(ctx: ServerContext) -> list[dict[str, Any]]:
    with _server_content_module()._connect(ctx) as conn:
        _server_content_module()._ensure_schema(conn, ctx)
        _ensure_default_repository_group(conn)
        _sync_repository_group_memberships(conn)
        conn.commit()
        groups = _list_repository_group_rows(conn)
        memberships = [
            dict(row)
            for row in conn.execute(
                """
                select coalesce(r.repo_name, m.repo_name) as repo_name, m.repo_id, m.group_id, m.sort_index
                from repository_group_memberships m
                left join repositories r on r.repo_id = m.repo_id
                order by m.group_id asc, m.sort_index asc, coalesce(m.repo_id, m.repo_name) asc
                """
            ).fetchall()
        ]
    repo_names_by_group: dict[str, list[str]] = {str(group["group_id"]): [] for group in groups}
    for row in memberships:
        repo_names_by_group.setdefault(str(row["group_id"]), []).append(str(row["repo_name"]))
    return [
        {
            **group,
            "repo_names": list(repo_names_by_group.get(str(group["group_id"]), [])),
            "is_main": str(group.get("system_slug") or "") == DEFAULT_REPOSITORY_GROUP_SYSTEM_SLUG,
        }
        for group in groups
    ]


def create_repository_group(ctx: ServerContext, title: str) -> dict[str, Any]:
    normalized_title = str(title or "").strip()
    if not normalized_title:
        raise ValueError("Group name is required.")

    with _server_content_module()._connect(ctx) as conn:
        _server_content_module()._ensure_schema(conn, ctx)
        _ensure_default_repository_group(conn)
        groups = _normalize_repository_group_order(conn)
        now = utc_now()
        group_id = _new_repository_group_id(normalized_title)
        conn.execute(
            """
            insert into repository_groups(group_id, title, sort_index, system_slug, created_at, updated_at)
            values (?, ?, ?, null, ?, ?)
            """,
            (group_id, normalized_title, len(groups) + 1, now, now),
        )
        conn.commit()
        row = conn.execute("select * from repository_groups where group_id = ?", (group_id,)).fetchone()
    assert row is not None
    out = _repository_group_row_out(row)
    out["repo_names"] = []
    out["is_main"] = False
    return out


def replace_repository_group_layout(ctx: ServerContext, groups: list[dict[str, Any]]) -> list[dict[str, Any]]:
    with _server_content_module()._connect(ctx) as conn:
        _server_content_module()._ensure_schema(conn, ctx)
        main_group = _ensure_default_repository_group(conn)
        current_groups = _list_repository_group_rows(conn)
        group_by_id = {str(group["group_id"]): group for group in current_groups}
        repo_rows = [
            dict(row)
            for row in conn.execute("select repo_name, repo_id from repositories order by repo_name asc").fetchall()
        ]
        repo_names = [str(row["repo_name"]) for row in repo_rows]
        repo_ids_by_name = {str(row["repo_name"]): str(row.get("repo_id") or "").strip() or None for row in repo_rows}
        known_repos = set(repo_names)
        if groups is None:
            raise ValueError("Repository group layout payload is required.")

        ordered_group_ids: list[str] = []
        group_repo_names: dict[str, list[str]] = {}
        seen_repos: set[str] = set()

        for entry in groups:
            if not isinstance(entry, dict):
                raise ValueError("Each repository group layout entry must be an object.")
            group_id = str(entry.get("group_id") or "").strip()
            if not group_id or group_id not in group_by_id:
                raise ValueError(f"Unknown repository group: {group_id or '<empty>'}")
            if group_id not in ordered_group_ids:
                ordered_group_ids.append(group_id)
            repos = entry.get("repo_names") or entry.get("repositories") or []
            if not isinstance(repos, list):
                raise ValueError("Repository group repo_names must be a list.")
            bucket = group_repo_names.setdefault(group_id, [])
            for repo_name in repos:
                normalized_repo_name = str(repo_name or "").strip()
                if not normalized_repo_name:
                    continue
                if normalized_repo_name not in known_repos:
                    raise ValueError(f"Unknown repository: {normalized_repo_name}")
                if normalized_repo_name in seen_repos:
                    raise ValueError(f"Repository appears more than once in layout: {normalized_repo_name}")
                seen_repos.add(normalized_repo_name)
                bucket.append(normalized_repo_name)

        if main_group["group_id"] not in ordered_group_ids:
            ordered_group_ids.insert(0, main_group["group_id"])
        for group in current_groups:
            group_id = str(group["group_id"])
            if group_id not in ordered_group_ids:
                ordered_group_ids.append(group_id)
            group_repo_names.setdefault(group_id, [])

        missing_repo_names = [repo_name for repo_name in repo_names if repo_name not in seen_repos]
        group_repo_names.setdefault(main_group["group_id"], []).extend(missing_repo_names)

        now = utc_now()
        for index, group_id in enumerate(ordered_group_ids, start=1):
            conn.execute(
                "update repository_groups set sort_index = ?, updated_at = ? where group_id = ?",
                (index, now, group_id),
            )

        conn.execute("delete from repository_group_memberships")
        membership_rows: list[tuple[str, str | None, str, int, str, str]] = []
        for group_id in ordered_group_ids:
            for index, repo_name in enumerate(group_repo_names.get(group_id, []), start=1):
                membership_rows.append((repo_name, repo_ids_by_name.get(repo_name), group_id, index, now, now))
        if membership_rows:
            conn.executemany(
                """
                insert into repository_group_memberships(repo_name, repo_id, group_id, sort_index, created_at, updated_at)
                values (?, ?, ?, ?, ?, ?)
                """,
                membership_rows,
            )
        _normalize_repository_group_memberships(conn)
        conn.commit()
    return list_repository_groups(ctx)




__all__ = [
    "_repository_group_row_out",
    "_new_repository_group_id",
    "_list_repository_group_rows",
    "_normalize_repository_group_order",
    "_normalize_repository_group_memberships",
    "_ensure_default_repository_group",
    "_sync_repository_group_memberships",
    "list_repository_groups",
    "create_repository_group",
    "replace_repository_group_layout",
]
