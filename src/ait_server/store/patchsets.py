from __future__ import annotations

import json
from typing import Any

from ait_protocol.common import AuthorMode, derive_patchset_id, normalize_author_mode, utc_now

from ..server_content import (
    connect as connect_content,
    get_snapshot_repo,
    snapshot_manifest_map,
)
from ..server_control import connect, record_event
from ..server_paths import ServerContext
from .repo_scoped_keys import _assert_repo_scope, _repo_scoped_sequence_ref
from .repo_ops import _repo_id, _repo_id_namespace_prefix
from .workflow_artifacts import _attestation_id_for_patchset, _invalidate_patchset_policy


def _legacy_server_store_module():
    from .. import server_store as legacy_server_store

    return legacy_server_store


def _ensure_change_mutable(*args, **kwargs):
    return _legacy_server_store_module()._ensure_change_mutable(*args, **kwargs)


def _normalize_optional_text(*args, **kwargs):
    return _legacy_server_store_module()._normalize_optional_text(*args, **kwargs)


def _refresh_change_state(*args, **kwargs):
    return _legacy_server_store_module()._refresh_change_state(*args, **kwargs)


def get_change_for_repo(*args, **kwargs):
    return _legacy_server_store_module().get_change_for_repo(*args, **kwargs)


def _diff_stats(ctx: ServerContext, base_snapshot_id: str, revision_snapshot_id: str) -> dict:
    base_map = snapshot_manifest_map(ctx, base_snapshot_id)
    rev_map = snapshot_manifest_map(ctx, revision_snapshot_id)
    base_paths = set(base_map)
    rev_paths = set(rev_map)
    added = sorted(rev_paths - base_paths)
    deleted = sorted(base_paths - rev_paths)
    modified = sorted(path for path in base_paths & rev_paths if base_map[path] != rev_map[path])
    return {
        "files_added": len(added),
        "files_deleted": len(deleted),
        "files_modified": len(modified),
        "files_changed": len(added) + len(deleted) + len(modified),
        "paths": {"added": added, "deleted": deleted, "modified": modified},
    }


def _snapshot_is_ancestor(ctx: ServerContext, ancestor_snapshot_id: str, descendant_snapshot_id: str) -> bool:
    resolved_ancestor_snapshot_id = _normalize_optional_text(ancestor_snapshot_id)
    resolved_descendant_snapshot_id = _normalize_optional_text(descendant_snapshot_id)
    if resolved_ancestor_snapshot_id is None or resolved_descendant_snapshot_id is None:
        return False
    if resolved_ancestor_snapshot_id == resolved_descendant_snapshot_id:
        return True
    with connect_content(ctx) as conn:
        seen: set[str] = set()
        current_snapshot_id = resolved_descendant_snapshot_id
        while current_snapshot_id and current_snapshot_id not in seen:
            seen.add(current_snapshot_id)
            row = conn.execute(
                "select parent_snapshot_id from snapshots where snapshot_id = ?",
                (current_snapshot_id,),
            ).fetchone()
            if row is None:
                return False
            parent_snapshot_id = _normalize_optional_text(row["parent_snapshot_id"])
            if parent_snapshot_id == resolved_ancestor_snapshot_id:
                return True
            current_snapshot_id = parent_snapshot_id
    return False


def publish_patchset(
    ctx: ServerContext,
    change_id: str,
    base_snapshot_id: str,
    revision_snapshot_id: str,
    summary: str,
    author_mode: str | AuthorMode,
) -> dict:
    with connect(ctx) as conn:
        try:
            change = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
            if change is None:
                raise KeyError(f"Unknown change: {change_id}")
            _ensure_change_mutable(change, "publish patchsets")
            repo_name = change["repo_name"]
            base_repo = get_snapshot_repo(ctx, base_snapshot_id)
            rev_repo = get_snapshot_repo(ctx, revision_snapshot_id)
            if base_repo != repo_name:
                raise KeyError(f"Unknown base snapshot: {base_snapshot_id}")
            if rev_repo != repo_name:
                raise KeyError(f"Unknown revision snapshot: {revision_snapshot_id}")
            if not _snapshot_is_ancestor(ctx, base_snapshot_id, revision_snapshot_id):
                raise ValueError(
                    f"Revision snapshot `{revision_snapshot_id}` does not descend from base snapshot "
                    f"`{base_snapshot_id}` for change `{change_id}`."
                )

            next_num = change["current_patchset_number"] + 1
            patchset_id = derive_patchset_id(change_id, next_num, _repo_id_namespace_prefix(ctx, repo_name))
            diff_stats = _diff_stats(ctx, base_snapshot_id, revision_snapshot_id)
            now = utc_now()
            author_mode_value = normalize_author_mode(author_mode)
            repo_id = str(change["repo_id"] or "").strip() or _repo_id(ctx, repo_name)
            if next_num > 1:
                conn.execute(
                    "update patchsets set publish_state = 'superseded' where change_id = ? and patchset_number = ?",
                    (change_id, next_num - 1),
                )
            conn.execute(
                "insert into patchsets(patchset_id, repo_id, change_id, patchset_number, base_snapshot_id, revision_snapshot_id, summary, author_mode, publish_state, diff_stats_json, evaluation_state, created_at) values (?, ?, ?, ?, ?, ?, ?, ?, 'published', ?, 'pending', ?)",
                (
                    patchset_id,
                    repo_id,
                    change_id,
                    next_num,
                    base_snapshot_id,
                    revision_snapshot_id,
                    summary,
                    author_mode_value,
                    json.dumps(diff_stats, sort_keys=True),
                    now,
                ),
            )
            conn.execute(
                "update changes set current_patchset_number = ?, status = 'review', updated_at = ?, selected_patchset_number = coalesce(selected_patchset_number, ?) where change_id = ?",
                (next_num, now, next_num, change_id),
            )
            record_event(
                conn,
                "patchset.published",
                "patchset",
                patchset_id,
                {
                    "change_id": change_id,
                    "patchset_number": next_num,
                    "base_snapshot_id": base_snapshot_id,
                    "revision_snapshot_id": revision_snapshot_id,
                },
            )
            conn.commit()
        except Exception:
            conn.rollback()
            raise

    with connect(ctx) as conn:
        _refresh_change_state(ctx, conn, change_id)
        conn.commit()
        row = conn.execute("select * from patchsets where patchset_id = ?", (patchset_id,)).fetchone()

    out = dict(row)
    out["diff_stats"] = diff_stats
    return out


def list_patchsets(ctx: ServerContext, change_id: str) -> list[dict]:
    with connect(ctx) as conn:
        if conn.execute("select 1 from changes where change_id = ?", (change_id,)).fetchone() is None:
            raise KeyError(f"Unknown change: {change_id}")
        rows = [dict(r) for r in conn.execute("select * from patchsets where change_id = ? order by patchset_number desc", (change_id,))]
    for row in rows:
        row["diff_stats"] = json.loads(row["diff_stats_json"])
    return rows


def list_patchsets_for_repo(ctx: ServerContext, repo_name: str, change_ref: str) -> list[dict]:
    change = get_change_for_repo(ctx, repo_name, change_ref)
    return list_patchsets(ctx, change["change_id"])


def get_patchset_for_repo(ctx: ServerContext, repo_name: str, patchset_ref: str, *, change_ref: str | None = None) -> dict:
    repo_id = _assert_repo_scope(ctx, repo_name)
    with connect(ctx) as conn:
        row = conn.execute(
            """
            select p.*
            from patchsets p
            join changes c on c.change_id = p.change_id
            where c.repo_id = ? and p.patchset_id = ?
            """,
            (repo_id, patchset_ref),
        ).fetchone()
        if row is None:
            patchset_number = _repo_scoped_sequence_ref(patchset_ref)
            if patchset_number is not None:
                if not change_ref:
                    raise KeyError(
                        f"Patchset ref {patchset_ref} for repository {repo_name} requires change_ref when using a local patchset number"
                    )
                change = get_change_for_repo(ctx, repo_name, change_ref)
                row = conn.execute(
                    "select * from patchsets where change_id = ? and patchset_number = ?",
                    (change["change_id"], patchset_number),
                ).fetchone()
    if row is None:
        raise KeyError(f"Unknown patchset {patchset_ref} for repository {repo_name}")
    out = dict(row)
    out["diff_stats"] = json.loads(out["diff_stats_json"])
    return out


def get_patchset(ctx: ServerContext, patchset_id: str) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select * from patchsets where patchset_id = ?", (patchset_id,)).fetchone()
    if row is None:
        raise KeyError(f"Unknown patchset: {patchset_id}")
    out = dict(row)
    out["diff_stats"] = json.loads(out["diff_stats_json"])
    return out


def select_patchset(ctx: ServerContext, change_id: str, patchset_id: str) -> dict:
    with connect(ctx) as conn:
        change = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
        if change is None:
            raise KeyError(f"Unknown change: {change_id}")
        _ensure_change_mutable(change, "select patchsets")
        patchset = conn.execute("select * from patchsets where patchset_id = ? and change_id = ?", (patchset_id, change_id)).fetchone()
        if patchset is None:
            raise KeyError(f"Patchset {patchset_id} does not belong to change {change_id}")
        conn.execute(
            "update patchsets set publish_state = case when patchset_id = ? then 'selected_for_landing' when publish_state = 'selected_for_landing' then 'published' else publish_state end where change_id = ?",
            (patchset_id, change_id),
        )
        conn.execute(
            "update changes set selected_patchset_number = ?, updated_at = ? where change_id = ?",
            (patchset["patchset_number"], utc_now(), change_id),
        )
        record_event(conn, "patchset.selected", "patchset", patchset_id, {"change_id": change_id, "patchset_number": patchset["patchset_number"]})
        _refresh_change_state(ctx, conn, change_id)
        conn.commit()
        row = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
    out = dict(row)
    out["selected_patchset_id"] = patchset_id
    return out


def upsert_attestation(
    ctx: ServerContext,
    patchset_id: str,
    author_mode: str | AuthorMode,
    evaluation_summary: dict[str, Any],
    provenance_summary: dict[str, Any],
    detail: dict[str, Any] | None = None,
) -> dict:
    with connect(ctx) as conn:
        try:
            patchset = conn.execute("select * from patchsets where patchset_id = ?", (patchset_id,)).fetchone()
            if patchset is None:
                raise KeyError(f"Unknown patchset: {patchset_id}")
            attestation_id = _attestation_id_for_patchset(patchset_id)
            now = utc_now()
            existing = conn.execute("select * from attestations where patchset_id = ?", (patchset_id,)).fetchone()
            author_mode_value = normalize_author_mode(author_mode)
            repo_id = str(patchset["repo_id"] or "").strip()
            if existing is None:
                conn.execute(
                    "insert into attestations(attestation_id, repo_id, patchset_id, author_mode, evaluation_summary_json, provenance_summary_json, detail_json, created_at, updated_at) values (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                    (
                        attestation_id,
                        repo_id,
                        patchset_id,
                        author_mode_value,
                        json.dumps(evaluation_summary, sort_keys=True),
                        json.dumps(provenance_summary, sort_keys=True),
                        json.dumps(detail or {}, sort_keys=True),
                        now,
                        now,
                    ),
                )
                event_type = "attestation.created"
            else:
                conn.execute(
                    "update attestations set author_mode = ?, evaluation_summary_json = ?, provenance_summary_json = ?, detail_json = ?, updated_at = ? where patchset_id = ?",
                    (
                        author_mode_value,
                        json.dumps(evaluation_summary, sort_keys=True),
                        json.dumps(provenance_summary, sort_keys=True),
                        json.dumps(detail or {}, sort_keys=True),
                        now,
                        patchset_id,
                    ),
                )
                event_type = "attestation.updated"
            _invalidate_patchset_policy(conn, patchset_id)
            record_event(conn, event_type, "patchset", patchset_id, {"patchset_id": patchset_id, "author_mode": author_mode_value})
            conn.commit()
            row = conn.execute("select * from attestations where patchset_id = ?", (patchset_id,)).fetchone()
        except Exception:
            conn.rollback()
            raise

    return {
        "attestation_id": row["attestation_id"],
        "patchset_id": row["patchset_id"],
        "author_mode": row["author_mode"],
        "evaluation_summary": json.loads(row["evaluation_summary_json"]),
        "provenance_summary": json.loads(row["provenance_summary_json"]),
        "detail": json.loads(row["detail_json"] or "{}"),
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }


def get_attestation(ctx: ServerContext, patchset_id: str) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select * from attestations where patchset_id = ?", (patchset_id,)).fetchone()
    if row is None:
        raise KeyError(f"No attestation for patchset: {patchset_id}")
    return {
        "attestation_id": row["attestation_id"],
        "patchset_id": row["patchset_id"],
        "author_mode": row["author_mode"],
        "evaluation_summary": json.loads(row["evaluation_summary_json"]),
        "provenance_summary": json.loads(row["provenance_summary_json"]),
        "detail": json.loads(row["detail_json"] or "{}"),
        "created_at": row["created_at"],
        "updated_at": row["updated_at"],
    }
