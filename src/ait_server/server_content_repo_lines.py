from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Any, Optional

from ait_protocol.common import (
    DEFAULT_ID_NAMESPACE_PREFIX,
    encode_ref_name,
    normalize_id_namespace_prefix,
    normalize_policy,
    utc_now,
)

from .server_content_groups import (
    _ensure_default_repository_group,
    _sync_repository_group_memberships,
)
from .server_db import postgres_advisory_lock
from .server_paths import ServerContext

__all__ = [
    "_read_ref_for_repository",
    "_read_ref_from_paths",
    "_ref_paths",
    "_repo_ref_path",
    "_write_ref_for_repository",
    "archive_line",
    "ensure_repository",
    "get_line",
    "get_repository",
    "list_lines",
    "list_lines_by_head_snapshot_ids",
    "read_ref",
    "repository_exists",
    "set_repository_lifecycle_state",
    "update_line",
    "write_ref",
]


def _server_content_module():
    from . import server_content as _server_content

    return _server_content


def _repo_ref_path(ctx: ServerContext, repo_token: str, line_name: str) -> Path:
    return ctx.ref_root / encode_ref_name(repo_token) / "lines" / encode_ref_name(line_name)


def _ref_paths(ctx: ServerContext, repo_name: str, repo_id: str | None, line_name: str) -> tuple[Path, Path | None]:
    current = _repo_ref_path(ctx, repo_id or repo_name, line_name)
    legacy = None if repo_id is None or repo_id == repo_name else _repo_ref_path(ctx, repo_name, line_name)
    return current, legacy


def _read_ref_from_paths(current_path: Path, legacy_path: Path | None) -> str | None:
    for candidate in (current_path, legacy_path):
        if candidate is None or not candidate.exists():
            continue
        text = candidate.read_text(encoding="utf-8").strip()
        if text:
            return text
    return None


def _read_ref_for_repository(ctx: ServerContext, repo_name: str, repo_id: str | None, line_name: str) -> str | None:
    current_path, legacy_path = _ref_paths(ctx, repo_name, repo_id, line_name)
    return _read_ref_from_paths(current_path, legacy_path)


def _write_ref_for_repository(
    ctx: ServerContext,
    repo_name: str,
    repo_id: str | None,
    line_name: str,
    snapshot_id: str | None,
) -> None:
    path, legacy_path = _ref_paths(ctx, repo_name, repo_id, line_name)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text((snapshot_id or "") + "\n", encoding="utf-8")
    if legacy_path is not None and legacy_path.exists():
        legacy_path.unlink()


def read_ref(ctx: ServerContext, repo_name: str, line_name: str) -> str | None:
    module = _server_content_module()
    with module._connect(ctx) as conn:
        module._ensure_schema(conn, ctx)
        repo_id, _ = module._repository_scope_params(conn, repo_name)
    return _read_ref_for_repository(ctx, repo_name, repo_id, line_name)


def write_ref(ctx: ServerContext, repo_name: str, line_name: str, snapshot_id: str | None) -> None:
    module = _server_content_module()
    repo = get_repository(ctx, repo_name)
    repo_id = str(repo.get("repo_id") or "").strip() or None
    _write_ref_for_repository(ctx, repo_name, repo_id, line_name, snapshot_id)
    with module._connect(ctx) as conn:
        module._ensure_schema(conn, ctx)
        scoped_repo_id, scoped_repo_name = module._repository_scope_params(conn, repo_name)
        module._set_line_head_snapshot_id(
            conn,
            repo_id=scoped_repo_id,
            scoped_repo_name=scoped_repo_name,
            line_name=line_name,
            head_snapshot_id=module._normalize_head_snapshot_id(snapshot_id),
        )
        conn.commit()


def ensure_repository(
    ctx: ServerContext,
    repo_name: str,
    default_line: str,
    policy: dict[str, Any] | None = None,
    *,
    id_namespace_prefix: str | None = None,
) -> dict:
    module = _server_content_module()
    with module._connect(ctx) as conn:
        module._ensure_schema(conn, ctx)
        row = conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone()
        now = utc_now()
        normalized_policy = normalize_policy(policy) if policy is not None else None
        normalized_prefix = (
            normalize_id_namespace_prefix(id_namespace_prefix, default=DEFAULT_ID_NAMESPACE_PREFIX)
            if id_namespace_prefix is not None
            else None
        )
        if row is None:
            stored_policy = normalized_policy or normalize_policy(None)
            stored_prefix = (
                normalized_prefix
                if normalized_prefix is not None
                else module._derived_repository_namespace_prefix(repo_name)
            )
            repo_id = module._new_repository_id()
            conflict = module._repository_namespace_prefix_conflict(conn, stored_prefix)
            if conflict is not None:
                module._raise_repository_namespace_prefix_conflict(stored_prefix, str(conflict["repo_name"]))
            conn.execute(
                """
                insert into repositories(repo_name, repo_id, default_line, id_namespace_prefix, policy_json, created_at, updated_at)
                values (?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    repo_name,
                    repo_id,
                    default_line,
                    stored_prefix,
                    json.dumps(stored_policy, sort_keys=True),
                    now,
                    now,
                ),
            )
            conn.execute(
                "insert into lines(repo_name, repo_id, line_name, created_at, updated_at) values (?, ?, ?, ?, ?)",
                (repo_name, repo_id, default_line, now, now),
            )
            module._bootstrap_authority_files_not_seeded(ctx, repo_name)
            conn.commit()
            row = conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone()
            write_ref(ctx, repo_name, default_line, None)
        else:
            existing = module._repository_out(row)
            updates: list[str] = []
            parameters: list[Any] = []
            if normalized_policy is not None and existing["policy"] != normalized_policy:
                updates.append("policy_json = ?")
                parameters.append(json.dumps(normalized_policy, sort_keys=True))
            if normalized_prefix is not None and existing.get("id_namespace_prefix") != normalized_prefix:
                conflict = module._repository_namespace_prefix_conflict(
                    conn,
                    normalized_prefix,
                    exclude_repo_name=repo_name,
                )
                if conflict is not None:
                    module._raise_repository_namespace_prefix_conflict(normalized_prefix, str(conflict["repo_name"]))
                updates.append("id_namespace_prefix = ?")
                parameters.append(normalized_prefix)
            if updates:
                updates.append("updated_at = ?")
                parameters.append(now)
                parameters.append(repo_name)
                conn.execute(
                    f"update repositories set {', '.join(updates)} where repo_name = ?",
                    tuple(parameters),
                )
                conn.commit()
                row = conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone()
        _ensure_default_repository_group(conn)
        _sync_repository_group_memberships(conn)
        conn.commit()
    return module._repository_out(row)


def repository_exists(ctx: ServerContext, repo_name: str) -> bool:
    module = _server_content_module()
    with module._connect(ctx) as conn:
        module._ensure_schema(conn, ctx)
        row = conn.execute("select 1 from repositories where repo_name = ?", (repo_name,)).fetchone()
    return row is not None


def get_repository(ctx: ServerContext, repo_name: str) -> dict:
    module = _server_content_module()
    with module._connect(ctx) as conn:
        module._ensure_schema(conn, ctx)
        row = conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone()
    if row is None:
        raise KeyError(f"Unknown repository: {repo_name}")
    return module._repository_out(row)


def set_repository_lifecycle_state(
    ctx: ServerContext,
    repo_name: str,
    lifecycle_state: str,
    *,
    expected_repo_id: str | None = None,
) -> dict:
    module = _server_content_module()
    normalized_state = str(lifecycle_state or "").strip().lower()
    if normalized_state not in module.REPOSITORY_LIFECYCLE_STATES:
        raise ValueError(f"Unsupported repository lifecycle state: {lifecycle_state!r}")
    with module._connect(ctx) as conn:
        module._ensure_schema(conn, ctx)
        row = conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone()
        if row is None:
            raise KeyError(f"Unknown repository: {repo_name}")
        current = module._repository_out(row)
        normalized_expected_repo_id = str(expected_repo_id or "").strip()
        if normalized_expected_repo_id and normalized_expected_repo_id != current["repo_id"]:
            raise ValueError(
                f"Repository scope mismatch for {repo_name}: repo_id {normalized_expected_repo_id} does not match {current['repo_id']}"
            )
        if current["lifecycle_state"] == normalized_state:
            return current
        conn.execute(
            "update repositories set lifecycle_state = ?, updated_at = ? where repo_name = ?",
            (normalized_state, utc_now(), repo_name),
        )
        conn.commit()
        row = conn.execute("select * from repositories where repo_name = ?", (repo_name,)).fetchone()
    assert row is not None
    return module._repository_out(row)


def list_lines(ctx: ServerContext, repo_name: str) -> list[dict]:
    module = _server_content_module()
    with module._connect(ctx) as conn:
        module._ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = module._repository_scope_params(conn, repo_name)
        rows = [
            dict(r)
            for r in conn.execute(
                "select * from lines where " + module._repository_id_scope_predicate() + " order by line_name",
                (repo_id, scoped_repo_name),
            )
        ]
    for row in rows:
        row["status"] = row.get("status") or "active"
        resolved_repo_id = str(row.get("repo_id") or repo_id or "").strip() or None
        row["head_snapshot_id"] = _read_ref_for_repository(ctx, repo_name, resolved_repo_id, row["line_name"])
    return rows


def get_line(ctx: ServerContext, repo_name: str, line_name: str) -> dict:
    module = _server_content_module()
    with module._connect(ctx) as conn:
        module._ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = module._repository_scope_params(conn, repo_name)
        row = conn.execute(
            "select * from lines where " + module._repository_id_scope_predicate() + " and line_name = ?",
            (repo_id, scoped_repo_name, line_name),
        ).fetchone()
    if row is None:
        raise KeyError(f"Unknown line {line_name} for repository {repo_name}")
    out = dict(row)
    out["status"] = out.get("status") or "active"
    resolved_repo_id = str(out.get("repo_id") or "").strip() or None
    out["head_snapshot_id"] = _read_ref_for_repository(ctx, repo_name, resolved_repo_id, line_name)
    return out


def list_lines_by_head_snapshot_ids(
    ctx: ServerContext,
    repo_name: str,
    head_snapshot_ids: set[str] | list[str] | tuple[str, ...],
    *,
    exclude_line_names: set[str] | list[str] | tuple[str, ...] | None = None,
) -> list[dict]:
    module = _server_content_module()
    normalized_snapshot_ids = sorted(
        item
        for item in {module._normalize_head_snapshot_id(item) for item in head_snapshot_ids if item}
        if item is not None
    )
    if not normalized_snapshot_ids:
        return []
    normalized_excluded_line_names = sorted({str(item).strip() for item in exclude_line_names or [] if str(item).strip()})
    with module._connect(ctx) as conn:
        module._ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = module._repository_scope_params(conn, repo_name)
        snapshot_placeholders = ", ".join("?" for _ in normalized_snapshot_ids)
        exclude_clause = ""
        query_params: list[Any] = [repo_id, scoped_repo_name, *normalized_snapshot_ids]
        if normalized_excluded_line_names:
            exclude_placeholders = ", ".join("?" for _ in normalized_excluded_line_names)
            exclude_clause = f" and line_name not in ({exclude_placeholders})"
            query_params.extend(normalized_excluded_line_names)
        rows = [
            dict(r)
            for r in conn.execute(
                "select * from lines where "
                + module._repository_id_scope_predicate()
                + f" and head_snapshot_id in ({snapshot_placeholders}){exclude_clause} order by line_name",
                tuple(query_params),
            )
        ]
    for row in rows:
        row["status"] = row.get("status") or "active"
        row["head_snapshot_id"] = module._normalize_head_snapshot_id(row.get("head_snapshot_id"))
    return rows


def update_line(
    ctx: ServerContext,
    repo_name: str,
    line_name: str,
    head_snapshot_id: Optional[str],
    *,
    expected_head_snapshot_id: Optional[str] = None,
    timings: Optional[dict[str, Any]] = None,
) -> dict:
    module = _server_content_module()

    def _update(conn):
        module._ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = module._repository_scope_params(conn, repo_name)
        resolved_repo_id = str(repo_id or "").strip() or None
        lock_scope = f"{ctx.content_schema or 'content'}:line:{resolved_repo_id or scoped_repo_name}:{line_name}"
        total_started = time.perf_counter()
        lock_wait_started = total_started
        lock_hold_started: float | None = None
        try:
            with postgres_advisory_lock(conn, scope=lock_scope):
                lock_hold_started = time.perf_counter()
                row = conn.execute(
                    "select * from lines where " + module._repository_id_scope_predicate() + " and line_name = ?",
                    (repo_id, scoped_repo_name, line_name),
                ).fetchone()
                current_head_snapshot_id = _read_ref_for_repository(ctx, repo_name, resolved_repo_id, line_name)
                if expected_head_snapshot_id is not None and current_head_snapshot_id != expected_head_snapshot_id:
                    raise ValueError(
                        f"Line {line_name} head advanced before update: "
                        f"expected {expected_head_snapshot_id!r}, got {current_head_snapshot_id!r}"
                    )
                now = utc_now()
                if row is None:
                    conn.execute(
                        "insert into lines(repo_name, repo_id, line_name, head_snapshot_id, status, archived_at, created_at, updated_at) "
                        "values (?, ?, ?, ?, 'active', null, ?, ?)",
                        (repo_name, repo_id, line_name, head_snapshot_id, now, now),
                    )
                else:
                    if (row["status"] or "active") == "archived":
                        raise ValueError(f"Line {line_name} is archived and cannot move")
                    conn.execute(
                        "update lines set head_snapshot_id = ?, updated_at = ? where "
                        + module._repository_id_scope_predicate()
                        + " and line_name = ?",
                        (head_snapshot_id, now, repo_id, scoped_repo_name, line_name),
                    )
                _write_ref_for_repository(ctx, repo_name, resolved_repo_id, line_name, head_snapshot_id)
                refreshed = conn.execute(
                    "select * from lines where " + module._repository_id_scope_predicate() + " and line_name = ?",
                    (repo_id, scoped_repo_name, line_name),
                ).fetchone()
                assert refreshed is not None
                out = dict(refreshed)
                out["status"] = out.get("status") or "active"
                out["head_snapshot_id"] = head_snapshot_id
                return out
        finally:
            if timings is not None:
                finished = time.perf_counter()
                if lock_hold_started is None:
                    timings["advisory_lock_wait"] = module._elapsed_ms(lock_wait_started, finished)
                else:
                    timings["advisory_lock_wait"] = module._elapsed_ms(lock_wait_started, lock_hold_started)
                    timings["advisory_lock_hold"] = module._elapsed_ms(lock_hold_started, finished)
                timings["total"] = module._elapsed_ms(total_started, finished)

    return module.write(ctx, _update)


def archive_line(ctx: ServerContext, repo_name: str, line_name: str) -> dict:
    module = _server_content_module()
    with module._connect(ctx) as conn:
        module._ensure_schema(conn, ctx)
        repo_id, scoped_repo_name = module._repository_scope_params(conn, repo_name)
        row = conn.execute(
            "select * from lines where " + module._repository_id_scope_predicate() + " and line_name = ?",
            (repo_id, scoped_repo_name, line_name),
        ).fetchone()
        if row is None:
            raise KeyError(f"Unknown line {line_name} for repository {repo_name}")
        if (row["status"] or "active") == "archived":
            return get_line(ctx, repo_name, line_name)
        now = utc_now()
        conn.execute(
            "update lines set status = 'archived', archived_at = ?, updated_at = ? "
            "where " + module._repository_id_scope_predicate() + " and line_name = ?",
            (now, now, repo_id, scoped_repo_name, line_name),
        )
        conn.commit()
    return get_line(ctx, repo_name, line_name)
