from __future__ import annotations

import hashlib
import json
import re
from typing import Any

from ait_protocol.common import (
    find_plan_item_in_items,
    generate_namespaced_workflow_id,
    normalize_plan_items,
    utc_now,
)

from ..server_content import read_blob_bytes, repository_exists, write_blob_bytes
from ..server_control import connect, record_event
from ..server_paths import ServerContext
from .sessions import create_session, list_sessions

PLAN_STATUSES = {"draft", "active", "superseded", "archived"}
PLANNING_SESSION_STATUSES = {"active", "closed"}
PLANNING_SESSION_ARTIFACT_STATUSES = {"not_promoted", "promoted"}
TASK_GRAPH_ARTIFACT_ROLE = "task_graph_json"
_PLAN_LINK_LIST_ITEM_RE = re.compile(r"^(?P<indent>\s*)(?:[-*+]|\d+\.)\s+(?:\[[ xX]\]\s+)?")


def _repo_id_namespace_prefix(ctx: ServerContext, repo_name: str) -> str:
    from .repo_ops import _repo_id_namespace_prefix as repo_id_namespace_prefix

    return repo_id_namespace_prefix(ctx, repo_name)


def _repo_id(ctx: ServerContext, repo_name: str) -> str:
    from .repo_ops import _repo_id as repo_id_for_repo

    return repo_id_for_repo(ctx, repo_name)


def _local_id_after_first_dash(value: str | None) -> str | None:
    text = str(value or "").strip()
    if not text:
        return None
    if "-" not in text:
        return text
    suffix = text.split("-", 1)[1].strip()
    return suffix or text


def _normalize_nonempty_text(value: str | None, *, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} must not be empty")
    return text


def _normalize_optional_text(value: str | None) -> str | None:
    text = str(value or "").strip()
    return text or None


def _normalize_plan_revision_artifact(
    *,
    artifact_path: str | None,
    artifact_selector: str | None,
    artifact_heading: str | None,
    items: list[dict[str, Any]] | None,
) -> tuple[str, str | None, str, list[dict[str, Any]]]:
    normalized_path = _normalize_nonempty_text(artifact_path, field="Plan artifact_path")
    normalized_selector = _normalize_optional_text(artifact_selector)
    normalized_heading = _normalize_nonempty_text(artifact_heading, field="Plan artifact_heading")
    normalized_items = normalize_plan_items(items)
    return normalized_path, normalized_selector, normalized_heading, normalized_items

def _normalize_plan_status(status: str | None) -> str:
    value = _normalize_nonempty_text(status or "draft", field="Plan status")
    if value not in PLAN_STATUSES:
        expected = ", ".join(sorted(PLAN_STATUSES))
        raise ValueError(f"Unsupported plan status: {value}. Expected one of: {expected}")
    return value

def _normalize_actor_identity(value: str | None) -> str:
    return _normalize_optional_text(value) or "system"

def _normalize_actor_type(value: str | None) -> str:
    return _normalize_optional_text(value) or "system_worker"


def _normalize_planning_session_status(status: str | None) -> str:
    value = _normalize_nonempty_text(status or "active", field="Planning session status")
    if value not in PLANNING_SESSION_STATUSES:
        expected = ", ".join(sorted(PLANNING_SESSION_STATUSES))
        raise ValueError(f"Unsupported planning session status: {value}. Expected one of: {expected}")
    return value


def _normalize_planning_session_mode(mode: str | None) -> str:
    return _normalize_nonempty_text(mode or "connected_local", field="Planning session mode")


def _normalize_planning_session_artifact_status(status: str | None) -> str:
    value = _normalize_nonempty_text(status or "not_promoted", field="Planning session artifact_status")
    if value not in PLANNING_SESSION_ARTIFACT_STATUSES:
        expected = ", ".join(sorted(PLANNING_SESSION_ARTIFACT_STATUSES))
        raise ValueError(f"Unsupported planning session artifact_status: {value}. Expected one of: {expected}")
    return value


def _planning_session_row(row: dict[str, Any] | Any) -> dict[str, Any]:
    return dict(row) if not isinstance(row, dict) else dict(row)


def _planning_session_event_row(row: dict[str, Any] | Any) -> dict[str, Any]:
    out = dict(row) if not isinstance(row, dict) else dict(row)
    out["payload"] = json.loads(out.pop("payload_json") or "{}")
    return out


def _artifact_declares_task_graph(role: str | None, metadata_json: str | None) -> bool:
    if _normalize_optional_text(role) == TASK_GRAPH_ARTIFACT_ROLE:
        return True
    try:
        metadata = json.loads(metadata_json or "{}")
    except Exception:
        metadata = {}
    return _normalize_optional_text(metadata.get("artifact_kind")) == TASK_GRAPH_ARTIFACT_ROLE


def _plan_revision_has_task_graph_artifact(conn, plan_revision_id: str | None) -> bool:
    resolved_revision_id = _normalize_optional_text(plan_revision_id)
    if resolved_revision_id is None:
        return False
    rows = conn.execute(
        """
        select role, metadata_json
        from plan_revision_artifacts
        where plan_revision_id = ?
        order by created_at desc, artifact_path asc
        """,
        (resolved_revision_id,),
    ).fetchall()
    return any(_artifact_declares_task_graph(row["role"], row["metadata_json"]) for row in rows)


def _plan_link_inline_text(value: Any) -> str:
    return " ".join(str(value or "").split()).strip()


def _plan_link_heading_path(value: Any) -> list[str]:
    if not isinstance(value, list):
        return []
    out: list[str] = []
    for segment in value:
        normalized = _plan_link_inline_text(segment)
        if normalized:
            out.append(normalized)
    return out


def _plan_link_item_details_by_ref(markdown: str | None, items: list[dict[str, Any]] | None) -> dict[str, str]:
    lines = str(markdown or "").splitlines()
    details_by_ref: dict[str, str] = {}
    for item in items or []:
        if not isinstance(item, dict):
            continue
        plan_item_ref = _normalize_optional_text(item.get("plan_item_ref"))
        line_number = int(item.get("line_number") or 0)
        if plan_item_ref is None or line_number <= 0 or line_number > len(lines):
            continue
        line_index = line_number - 1
        raw_line = lines[line_index]
        list_match = _PLAN_LINK_LIST_ITEM_RE.match(raw_line)
        if list_match is None:
            continue
        item_indent = len(list_match.group("indent"))
        detail_lines: list[str] = []
        pending_blank = False
        for raw_following in lines[line_index + 1 :]:
            if raw_following.lstrip().startswith("#"):
                break
            following_match = _PLAN_LINK_LIST_ITEM_RE.match(raw_following)
            current_indent = len(raw_following) - len(raw_following.lstrip())
            if following_match is not None and current_indent <= item_indent:
                break
            if not raw_following.strip():
                pending_blank = True
                continue
            if current_indent <= item_indent and following_match is None:
                break
            if pending_blank and detail_lines:
                detail_lines.append("")
            detail_lines.append(raw_following.strip())
            pending_blank = False
        detail_text = "\n".join(detail_lines).strip()
        if detail_text:
            details_by_ref[plan_item_ref] = detail_text
    return details_by_ref


def _plan_link_surface_entries(
    items: list[dict[str, Any]] | None,
    *,
    artifact_body: str | None,
) -> dict[str, dict[str, Any]]:
    normalized_items = normalize_plan_items(items)
    details_by_ref = _plan_link_item_details_by_ref(artifact_body, normalized_items)
    entries: dict[str, dict[str, Any]] = {}
    for item in normalized_items:
        plan_item_ref = _normalize_optional_text(item.get("plan_item_ref"))
        if plan_item_ref is None:
            continue
        entries[plan_item_ref] = {
            "plan_item_ref": plan_item_ref,
            "text": _plan_link_inline_text(item.get("text")),
            "checkbox_state": _normalize_optional_text(item.get("checkbox_state")) or "none",
            "heading_path": _plan_link_heading_path(item.get("heading_path")),
            "details": _normalize_optional_text(details_by_ref.get(plan_item_ref)) or "",
        }
    return entries


def _plan_link_surface_hash(entries: dict[str, dict[str, Any]]) -> str:
    payload = [entries[plan_item_ref] for plan_item_ref in sorted(entries)]
    canonical = json.dumps(payload, ensure_ascii=True, separators=(",", ":"), sort_keys=True)
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


def _plan_link_changed_count(
    previous_entries: dict[str, dict[str, Any]] | None,
    current_entries: dict[str, dict[str, Any]],
) -> int:
    if previous_entries is None:
        return 0
    changed = 0
    for plan_item_ref in sorted(set(previous_entries) | set(current_entries)):
        if previous_entries.get(plan_item_ref) != current_entries.get(plan_item_ref):
            changed += 1
    return changed


def _plan_link_blob_markdown(conn, ctx: ServerContext, plan_revision_id: str) -> str | None:
    blob_row = conn.execute(
        """
        select blob_id, encoding
        from plan_revision_blobs
        where plan_revision_id = ?
        """,
        (plan_revision_id,),
    ).fetchone()
    if blob_row is None:
        return None
    blob_id = _normalize_optional_text(blob_row["blob_id"])
    if blob_id is None:
        return None
    try:
        payload = read_blob_bytes(ctx, blob_id)
    except Exception:
        return None
    encoding = _normalize_optional_text(blob_row["encoding"]) or "utf-8"
    try:
        return payload.decode(encoding)
    except Exception:
        try:
            return payload.decode("utf-8")
        except Exception:
            return None


def _plan_link_metadata_for_revision(
    conn,
    ctx: ServerContext,
    *,
    plan_revision_id: str,
    items: list[dict[str, Any]] | None,
    artifact_body: str | None = None,
) -> tuple[str, dict[str, dict[str, Any]]]:
    if artifact_body is None and conn is not None:
        artifact_body = _plan_link_blob_markdown(conn, ctx, plan_revision_id)
    entries = _plan_link_surface_entries(items, artifact_body=artifact_body)
    return _plan_link_surface_hash(entries), entries


def _ensure_plan_link_diff_metadata(
    conn,
    ctx: ServerContext,
    *,
    plan_id: str,
) -> None:
    rows = conn.execute(
        """
        select
            plan_revision_id,
            revision_number,
            items_json,
            plan_links_surface_hash,
            plan_links_changed_count_to_prev
        from plan_revisions
        where plan_id = ?
        order by revision_number asc
        """,
        (plan_id,),
    ).fetchall()
    if not rows:
        return
    needs_backfill = any(_normalize_optional_text(row["plan_links_surface_hash"]) is None for row in rows)
    if not needs_backfill:
        return

    previous_entries: dict[str, dict[str, Any]] | None = None
    for row in rows:
        try:
            items = json.loads(row["items_json"] or "[]")
        except Exception:
            items = []
        surface_hash, current_entries = _plan_link_metadata_for_revision(
            conn,
            ctx,
            plan_revision_id=row["plan_revision_id"],
            items=items,
        )
        changed_count = _plan_link_changed_count(previous_entries, current_entries)
        conn.execute(
            """
            update plan_revisions
            set plan_links_surface_hash = ?,
                plan_links_changed_count_to_prev = ?
            where plan_revision_id = ?
            """,
            (surface_hash, changed_count, row["plan_revision_id"]),
        )
        previous_entries = current_entries

def _plan_revision_view(
    row: dict[str, Any] | Any | None,
    conn=None,
    *,
    ctx: ServerContext | None = None,
    include_artifact_body: bool = False,
) -> dict[str, Any] | None:
    if row is None:
        return None
    out = dict(row) if not isinstance(row, dict) else dict(row)
    try:
        out["items"] = json.loads(out.pop("items_json", "[]") or "[]")
    except Exception:
        out["items"] = []
    out.setdefault("plan_links_surface_hash", None)
    try:
        out["plan_links_changed_count_to_prev"] = int(out.get("plan_links_changed_count_to_prev") or 0)
    except Exception:
        out["plan_links_changed_count_to_prev"] = 0
    out.setdefault("artifact_blob_id", None)
    if conn is not None and out.get("plan_revision_id"):
        artifact_body = None
        blob_row = conn.execute(
            """
            select blob_id, media_type, encoding, byte_count, created_at
            from plan_revision_blobs
            where plan_revision_id = ?
            """,
            (out["plan_revision_id"],),
        ).fetchone()
        if blob_row is not None:
            out["artifact_blob_id"] = blob_row["blob_id"]
            out["artifact_media_type"] = blob_row["media_type"]
            out["artifact_encoding"] = blob_row["encoding"]
            out["artifact_byte_count"] = blob_row["byte_count"]
            out["artifact_blob_created_at"] = blob_row["created_at"]
            if include_artifact_body and ctx is not None:
                artifact_body = _plan_link_blob_markdown(conn, ctx, str(out["plan_revision_id"]))
        else:
            out.setdefault("artifact_media_type", None)
            out.setdefault("artifact_encoding", None)
            out.setdefault("artifact_byte_count", None)
            out.setdefault("artifact_blob_created_at", None)
        out["artifacts"] = _plan_revision_artifact_views(conn, str(out["plan_revision_id"]))
        if include_artifact_body:
            out["artifact_body"] = artifact_body
    return out


def _plan_revision_artifact_views(conn, plan_revision_id: str) -> list[dict[str, Any]]:
    rows = conn.execute(
        """
        select
            plan_revision_id, artifact_path, repo_name, repo_id, role, blob_id,
            media_type, encoding, byte_count, sha256, metadata_json, created_at, updated_at
        from plan_revision_artifacts
        where plan_revision_id = ?
        order by role, artifact_path
        """,
        (plan_revision_id,),
    ).fetchall()
    out: list[dict[str, Any]] = []
    for row in rows:
        item = dict(row)
        try:
            item["metadata"] = json.loads(item.pop("metadata_json", "{}") or "{}")
        except Exception:
            item["metadata"] = {}
        out.append(item)
    return out


def _store_plan_revision_blob(
    conn,
    ctx: ServerContext,
    *,
    repo_name: str,
    repo_id: str,
    plan_revision_id: str,
    artifact_body: str | None,
    created_at: str,
) -> str | None:
    if artifact_body is None:
        return None
    data = artifact_body.encode("utf-8")
    blob = write_blob_bytes(
        ctx,
        repo_name,
        data,
        path_hint=f"plan-revisions/{plan_revision_id}.md",
        created_at=created_at,
    )
    conn.execute(
        """
        insert into plan_revision_blobs(
            plan_revision_id, repo_name, repo_id, blob_id, media_type, encoding, byte_count, created_at
        ) values (?, ?, ?, ?, 'text/markdown', 'utf-8', ?, ?)
        on conflict(plan_revision_id) do update set
            repo_name = excluded.repo_name,
            repo_id = excluded.repo_id,
            blob_id = excluded.blob_id,
            media_type = excluded.media_type,
            encoding = excluded.encoding,
            byte_count = excluded.byte_count,
            created_at = excluded.created_at
        """,
        (
            plan_revision_id,
            repo_name,
            repo_id,
            blob["blob_id"],
            len(data),
            created_at,
        ),
    )
    return str(blob["blob_id"])


def put_plan_revision_artifacts(
    ctx: ServerContext,
    plan_id: str,
    plan_revision_id: str,
    artifacts: list[dict[str, Any]],
    *,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict[str, Any]:
    if not artifacts:
        raise ValueError("At least one plan revision artifact is required.")
    normalized_actor_identity = _normalize_actor_identity(actor_identity)
    normalized_actor_type = _normalize_actor_type(actor_type)
    with connect(ctx) as conn:
        plan_row = _get_plan_row(conn, plan_id)
        revision_row = conn.execute(
            "select * from plan_revisions where plan_id = ? and plan_revision_id = ?",
            (plan_id, plan_revision_id),
        ).fetchone()
        if revision_row is None:
            raise KeyError(f"Unknown plan revision: {plan_revision_id}")
        repo_name = str(plan_row["repo_name"])
        repo_id = _normalize_optional_text(plan_row.get("repo_id")) or _repo_id(ctx, repo_name)
        now = utc_now()
        stored: list[dict[str, Any]] = []
        for artifact in artifacts:
            artifact_path = _normalize_nonempty_text(
                artifact.get("artifact_path"),
                field="Plan revision artifact path",
            )
            role = _normalize_nonempty_text(artifact.get("role") or "supporting_artifact", field="Plan revision artifact role")
            media_type = _normalize_nonempty_text(
                artifact.get("media_type") or "application/octet-stream",
                field="Plan revision artifact media_type",
            )
            encoding = _normalize_optional_text(artifact.get("encoding"))
            body = artifact.get("body")
            if not isinstance(body, str):
                raise ValueError(f"Plan revision artifact {artifact_path} body must be text.")
            data = body.encode(encoding or "utf-8")
            digest = hashlib.sha256(data).hexdigest()
            metadata = artifact.get("metadata") if isinstance(artifact.get("metadata"), dict) else {}
            blob = write_blob_bytes(
                ctx,
                repo_name,
                data,
                path_hint=f"plan-revisions/{plan_revision_id}/artifacts/{artifact_path}",
                created_at=now,
            )
            conn.execute(
                """
                insert into plan_revision_artifacts(
                    plan_revision_id, artifact_path, repo_name, repo_id, role, blob_id,
                    media_type, encoding, byte_count, sha256, metadata_json, created_at, updated_at
                ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                on conflict(plan_revision_id, artifact_path) do update set
                    repo_name = excluded.repo_name,
                    repo_id = excluded.repo_id,
                    role = excluded.role,
                    blob_id = excluded.blob_id,
                    media_type = excluded.media_type,
                    encoding = excluded.encoding,
                    byte_count = excluded.byte_count,
                    sha256 = excluded.sha256,
                    metadata_json = excluded.metadata_json,
                    updated_at = excluded.updated_at
                """,
                (
                    plan_revision_id,
                    artifact_path,
                    repo_name,
                    repo_id,
                    role,
                    blob["blob_id"],
                    media_type,
                    encoding,
                    len(data),
                    digest,
                    json.dumps(metadata, sort_keys=True),
                    now,
                    now,
                ),
            )
            stored.append(
                {
                    "plan_revision_id": plan_revision_id,
                    "artifact_path": artifact_path,
                    "repo_name": repo_name,
                    "repo_id": repo_id,
                    "role": role,
                    "blob_id": blob["blob_id"],
                    "media_type": media_type,
                    "encoding": encoding,
                    "byte_count": len(data),
                    "sha256": digest,
                    "metadata": metadata,
                    "created_at": now,
                    "updated_at": now,
                }
            )
        record_event(
            conn,
            "plan.revision_artifacts_put",
            "plan",
            plan_id,
            {
                "repo_name": repo_name,
                "plan_revision_id": plan_revision_id,
                "artifact_count": len(stored),
                "artifact_paths": [row["artifact_path"] for row in stored],
            },
            actor_identity=normalized_actor_identity,
            actor_type=normalized_actor_type,
        )
        conn.commit()
    return {"plan_id": plan_id, "plan_revision_id": plan_revision_id, "artifacts": stored}

def _plan_view(
    conn,
    row: dict[str, Any] | Any | None,
    *,
    include_head_revision: bool = True,
) -> dict[str, Any] | None:
    if row is None:
        return None
    out = dict(row) if not isinstance(row, dict) else dict(row)
    if include_head_revision:
        head_revision_id = out.get("head_revision_id")
        if head_revision_id:
            head_row = conn.execute(
                "select * from plan_revisions where plan_revision_id = ?",
                (head_revision_id,),
            ).fetchone()
            out["head_revision"] = _plan_revision_view(head_row, conn)
        else:
            out["head_revision"] = None
    return out

def _get_plan_row(conn, plan_id: str):
    row = conn.execute("select * from plans where plan_id = ?", (plan_id,)).fetchone()
    if row is None:
        raise KeyError(f"Unknown plan: {plan_id}")
    return row


def create_planning_session(
    ctx: ServerContext,
    plan_id: str,
    *,
    title: str | None = None,
    mode: str = "connected_local",
    preferred_agent: str | None = None,
    resume_if_active: bool = True,
    planning_session_id: str | None = None,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict:
    normalized_mode = _normalize_planning_session_mode(mode)
    normalized_title = _normalize_optional_text(title)
    normalized_preferred_agent = _normalize_optional_text(preferred_agent)
    normalized_actor_identity = _normalize_actor_identity(actor_identity)
    normalized_actor_type = _normalize_actor_type(actor_type)
    with connect(ctx) as conn:
        plan_row = _get_plan_row(conn, plan_id)
        repo_id = _repo_id(ctx, plan_row["repo_name"])
        if resume_if_active:
            active_row = conn.execute(
                """
                select * from planning_sessions
                where plan_id = ? and status = 'active'
                order by updated_at desc, created_at desc
                limit 1
                """,
                (plan_id,),
            ).fetchone()
            if active_row is not None:
                return _planning_session_row(active_row)
        repo_namespace_prefix = _repo_id_namespace_prefix(ctx, plan_row["repo_name"])
        if planning_session_id is None:
            planning_session_id = generate_namespaced_workflow_id("PS", repo_namespace_prefix)
        planning_session_local_id = _local_id_after_first_dash(planning_session_id)
        existing = conn.execute(
            "select * from planning_sessions where planning_session_id = ?",
            (planning_session_id,),
        ).fetchone()
        if existing is not None:
            row = dict(existing)
            if (
                row["repo_name"] == plan_row["repo_name"]
                and row["plan_id"] == plan_id
                and row.get("title") == normalized_title
                and row["mode"] == normalized_mode
                and row.get("preferred_agent") == normalized_preferred_agent
            ):
                return _planning_session_row(row)
            raise ValueError(f"Planning session {planning_session_id} already exists with different fields")
        now = utc_now()
        conn.execute(
            """
            insert into planning_sessions(
                planning_session_id, repo_name, repo_id, planning_session_local_id, plan_id, title, mode, status, preferred_agent, artifact_status,
                derived_task_id, last_promoted_plan_revision_id, last_event_sequence, created_by, created_at, updated_at
            ) values (?, ?, ?, ?, ?, ?, ?, 'active', ?, 'not_promoted', null, null, 0, ?, ?, ?)
            """,
            (
                planning_session_id,
                plan_row["repo_name"],
                repo_id,
                planning_session_local_id,
                plan_id,
                normalized_title,
                normalized_mode,
                normalized_preferred_agent,
                normalized_actor_identity,
                now,
                now,
            ),
        )
        record_event(
            conn,
            "planning_session.created",
            "planning_session",
            planning_session_id,
            {
                "repo_name": plan_row["repo_name"],
                "plan_id": plan_id,
                "mode": normalized_mode,
                "preferred_agent": normalized_preferred_agent,
            },
            actor_identity=normalized_actor_identity,
            actor_type=normalized_actor_type,
        )
        conn.commit()
        row = conn.execute(
            "select * from planning_sessions where planning_session_id = ?",
            (planning_session_id,),
        ).fetchone()
    assert row is not None
    return _planning_session_row(row)


def list_planning_sessions(ctx: ServerContext, plan_id: str, *, status: str | None = None) -> list[dict]:
    with connect(ctx) as conn:
        plan_row = _get_plan_row(conn, plan_id)
        if status is not None:
            normalized_status = _normalize_planning_session_status(status)
            rows = [
                dict(r)
                for r in conn.execute(
                    """
                    select * from planning_sessions
                    where plan_id = ? and status = ?
                    order by updated_at desc, created_at desc
                    """,
                    (plan_id, normalized_status),
                )
            ]
        else:
            rows = [
                dict(r)
                for r in conn.execute(
                    """
                    select * from planning_sessions
                    where plan_id = ?
                    order by updated_at desc, created_at desc
                    """,
                    (plan_id,),
                )
            ]
    if plan_row["repo_name"] is None:
        raise KeyError(f"Unknown plan: {plan_id}")
    return [_planning_session_row(row) for row in rows]


def get_planning_session(ctx: ServerContext, planning_session_id: str) -> dict:
    with connect(ctx) as conn:
        row = conn.execute(
            "select * from planning_sessions where planning_session_id = ?",
            (planning_session_id,),
        ).fetchone()
    if row is None:
        raise KeyError(f"Unknown planning session: {planning_session_id}")
    return _planning_session_row(row)


def append_planning_session_event(
    ctx: ServerContext,
    planning_session_id: str,
    event_type: str,
    payload: dict | None,
    *,
    actor_identity: str = "system",
    actor_type: str = "system_worker",
) -> dict:
    normalized_event_type = _normalize_nonempty_text(event_type, field="Planning session event_type")
    with connect(ctx) as conn:
        row = conn.execute(
            "select repo_name, repo_id, status, last_event_sequence from planning_sessions where planning_session_id = ?",
            (planning_session_id,),
        ).fetchone()
        if row is None:
            raise KeyError(f"Unknown planning session: {planning_session_id}")
        if row["status"] != "active":
            raise ValueError(f"Planning session {planning_session_id} is {row['status']} and cannot accept new events")
        next_sequence = int(row["last_event_sequence"] or 0) + 1
        now = utc_now()
        conn.execute(
            """
            insert into planning_session_events(
                repo_id, planning_session_id, sequence, event_type, payload_json, actor_identity, actor_type, created_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                row["repo_id"] or _repo_id(ctx, row["repo_name"]),
                planning_session_id,
                next_sequence,
                normalized_event_type,
                json.dumps(payload or {}, sort_keys=True),
                actor_identity,
                actor_type,
                now,
            ),
        )
        conn.execute(
            "update planning_sessions set last_event_sequence = ?, updated_at = ? where planning_session_id = ?",
            (next_sequence, now, planning_session_id),
        )
        record_event(
            conn,
            "planning_session.event_appended",
            "planning_session",
            planning_session_id,
            {"sequence": next_sequence, "event_type": normalized_event_type},
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        conn.commit()
        event_row = conn.execute(
            """
            select * from planning_session_events
            where planning_session_id = ? and sequence = ?
            """,
            (planning_session_id, next_sequence),
        ).fetchone()
    assert event_row is not None
    return _planning_session_event_row(event_row)


def list_planning_session_events(
    ctx: ServerContext,
    planning_session_id: str,
    *,
    after_sequence: int = 0,
    limit: int = 200,
) -> list[dict]:
    with connect(ctx) as conn:
        if conn.execute(
            "select 1 from planning_sessions where planning_session_id = ?",
            (planning_session_id,),
        ).fetchone() is None:
            raise KeyError(f"Unknown planning session: {planning_session_id}")
        rows = [
            dict(r)
            for r in conn.execute(
                """
                select * from planning_session_events
                where planning_session_id = ? and sequence > ?
                order by sequence asc
                limit ?
                """,
                (planning_session_id, max(int(after_sequence), 0), max(int(limit), 1)),
            )
        ]
    return [_planning_session_event_row(row) for row in rows]


def join_planning_session(
    ctx: ServerContext,
    planning_session_id: str,
    *,
    surface: str = "cli",
    title: str | None = None,
    model_name: str | None = None,
    resume_if_active: bool = True,
    session_id: str | None = None,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict[str, Any]:
    planning_session = get_planning_session(ctx, planning_session_id)
    if planning_session["status"] != "active":
        raise ValueError(f"Planning session {planning_session_id} is {planning_session['status']} and cannot accept relay joins")
    normalized_surface = _normalize_nonempty_text(surface or "cli", field="Planning session relay surface")
    normalized_title = _normalize_optional_text(title)
    normalized_model_name = _normalize_optional_text(model_name)
    normalized_actor_identity = _normalize_actor_identity(actor_identity)
    normalized_actor_type = _normalize_actor_type(actor_type)
    repo_name = str(planning_session["repo_name"])
    if resume_if_active:
        active_sessions = list_sessions(ctx, repo_name, status="active")
        for row in active_sessions:
            metadata = row.get("metadata") or {}
            if (
                row.get("session_kind") == "planning_session_relay"
                and str(metadata.get("planning_session_id") or "") == planning_session_id
                and str(metadata.get("surface") or "") == normalized_surface
            ):
                return {
                    "planning_session": planning_session,
                    "session": row,
                }
    session = create_session(
        ctx,
        repo_name,
        "planning_session_relay",
        title=normalized_title or planning_session.get("title") or f"planning session {planning_session_id}",
        model_name=normalized_model_name,
        metadata={
            "planning_session_id": planning_session_id,
            "plan_id": planning_session["plan_id"],
            "surface": normalized_surface,
            "preferred_agent": planning_session.get("preferred_agent"),
        },
        session_id=session_id,
        actor_identity=normalized_actor_identity,
        actor_type=normalized_actor_type,
    )
    return {
        "planning_session": planning_session,
        "session": session,
    }


def promote_planning_session(
    ctx: ServerContext,
    planning_session_id: str,
    artifact_path: str,
    artifact_selector: str,
    artifact_heading: str,
    items: list[dict[str, Any]],
    *,
    title: str | None = None,
    summary: str | None = None,
    artifact_body: str | None = None,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict:
    normalized_actor_identity = _normalize_actor_identity(actor_identity)
    normalized_actor_type = _normalize_actor_type(actor_type)
    session = get_planning_session(ctx, planning_session_id)
    if session["status"] != "active":
        raise ValueError(f"Planning session {planning_session_id} is {session['status']} and cannot be promoted")
    plan = revise_plan(
        ctx,
        session["plan_id"],
        artifact_path,
        artifact_selector,
        artifact_heading,
        items,
        title=title,
        summary=summary,
        source_kind="planning_session_promotion",
        source_session_id=planning_session_id,
        artifact_body=artifact_body,
        actor_identity=normalized_actor_identity,
        actor_type=normalized_actor_type,
    )
    promoted_revision = dict(plan.get("head_revision") or {})
    with connect(ctx) as conn:
        now = utc_now()
        conn.execute(
            """
            update planning_sessions
            set artifact_status = ?, last_promoted_plan_revision_id = ?, updated_at = ?
            where planning_session_id = ?
            """,
            ("promoted", promoted_revision.get("plan_revision_id"), now, planning_session_id),
        )
        record_event(
            conn,
            "planning_session.promoted",
            "planning_session",
            planning_session_id,
            {
                "plan_id": session["plan_id"],
                "promoted_plan_revision_id": promoted_revision.get("plan_revision_id"),
            },
            actor_identity=normalized_actor_identity,
            actor_type=normalized_actor_type,
        )
        conn.commit()
        row = conn.execute(
            "select * from planning_sessions where planning_session_id = ?",
            (planning_session_id,),
        ).fetchone()
    assert row is not None
    return {
        "planning_session": _planning_session_row(row),
        "plan": plan,
        "promoted_revision": promoted_revision,
    }


def close_planning_session(
    ctx: ServerContext,
    planning_session_id: str,
    *,
    status: str = "closed",
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict:
    normalized_status = _normalize_planning_session_status(status)
    normalized_actor_identity = _normalize_actor_identity(actor_identity)
    normalized_actor_type = _normalize_actor_type(actor_type)
    with connect(ctx) as conn:
        row = conn.execute(
            "select * from planning_sessions where planning_session_id = ?",
            (planning_session_id,),
        ).fetchone()
        if row is None:
            raise KeyError(f"Unknown planning session: {planning_session_id}")
        session = dict(row)
        if session["status"] == normalized_status:
            return _planning_session_row(session)
        now = utc_now()
        conn.execute(
            "update planning_sessions set status = ?, updated_at = ? where planning_session_id = ?",
            (normalized_status, now, planning_session_id),
        )
        record_event(
            conn,
            "planning_session.closed",
            "planning_session",
            planning_session_id,
            {
                "plan_id": session["plan_id"],
                "previous_status": session["status"],
                "status": normalized_status,
            },
            actor_identity=normalized_actor_identity,
            actor_type=normalized_actor_type,
        )
        conn.commit()
        row = conn.execute(
            "select * from planning_sessions where planning_session_id = ?",
            (planning_session_id,),
        ).fetchone()
    assert row is not None
    return _planning_session_row(row)


def _row_matches_repo_scope(row: dict[str, Any] | Any, *, repo_name: str, repo_id: str) -> bool:
    row_repo_id = _normalize_optional_text(row["repo_id"])
    if row_repo_id is not None:
        return row_repo_id == repo_id
    return _normalize_optional_text(row["repo_name"]) == repo_name

def _resolve_task_plan_linkage(
    ctx: ServerContext,
    conn,
    repo_name: str,
    *,
    plan_id: str | None,
    origin_plan_revision_id: str | None,
    plan_item_ref: str | None,
) -> tuple[str | None, str | None, str | None]:
    repo_id = _repo_id(ctx, repo_name)
    resolved_plan_id = _normalize_optional_text(plan_id)
    resolved_revision_id = _normalize_optional_text(origin_plan_revision_id)
    resolved_plan_item_ref = _normalize_optional_text(plan_item_ref)
    if resolved_plan_id is None and resolved_revision_id is None:
        if resolved_plan_item_ref is not None:
            raise ValueError("plan_item_ref requires plan linkage")
        return None, None, None

    plan_row = None
    if resolved_plan_id is not None:
        plan_row = _get_plan_row(conn, resolved_plan_id)
        if not _row_matches_repo_scope(plan_row, repo_name=repo_name, repo_id=repo_id):
            actual_repo = plan_row["repo_id"] or plan_row["repo_name"]
            raise KeyError(f"Plan {resolved_plan_id} belongs to repository {actual_repo}, not {repo_name}")

    revision_row = None
    if resolved_revision_id is not None:
        revision_row = conn.execute(
            """
            select pr.*, p.repo_name, p.repo_id
            from plan_revisions pr
            join plans p on p.plan_id = pr.plan_id
            where pr.plan_revision_id = ?
            """,
            (resolved_revision_id,),
        ).fetchone()
        if revision_row is None:
            raise KeyError(f"Unknown plan revision: {resolved_revision_id}")
        if not _row_matches_repo_scope(revision_row, repo_name=repo_name, repo_id=repo_id):
            actual_repo = revision_row["repo_id"] or revision_row["repo_name"]
            raise KeyError(f"Plan revision {resolved_revision_id} belongs to repository {actual_repo}, not {repo_name}")

    if plan_row is None and revision_row is not None:
        resolved_plan_id = revision_row["plan_id"]
        plan_row = _get_plan_row(conn, resolved_plan_id)
    elif plan_row is not None and revision_row is None:
        resolved_revision_id = plan_row["head_revision_id"]
        if not resolved_revision_id:
            raise ValueError(f"Plan {resolved_plan_id} has no head revision to link from")
        revision_row = conn.execute(
            "select * from plan_revisions where plan_revision_id = ?",
            (resolved_revision_id,),
        ).fetchone()
    if plan_row is not None and revision_row is not None and revision_row["plan_id"] != plan_row["plan_id"]:
        raise ValueError(
            f"Plan revision {resolved_revision_id} does not belong to plan {plan_row['plan_id']}"
        )
    if resolved_plan_item_ref is None and _plan_revision_has_task_graph_artifact(conn, resolved_revision_id):
        raise ValueError(
            f"Plan revision {resolved_revision_id} publishes a task DAG artifact, so plan-linked tasks must bind to a specific plan_item_ref. "
            "Use `ait plan execute ... --auto-compact-worker --yes`, "
            "`ait task start --plan-item-ref ...`, or pass `--plan-item-ref` to another surviving plan-linked task surface."
        )
    if resolved_plan_item_ref is not None:
        assert revision_row is not None
        revision = _plan_revision_view(revision_row) or {}
        plan_item = find_plan_item_in_items(revision.get("items"), resolved_plan_item_ref)
        if plan_item is None:
            known_refs = [item["plan_item_ref"] for item in revision.get("items") or []]
            if known_refs:
                raise ValueError(
                    f"Plan item ref {resolved_plan_item_ref!r} is not present in plan revision {resolved_revision_id}. "
                    f"Known refs: {', '.join(known_refs)}"
                )
            raise ValueError(
                f"Plan revision {resolved_revision_id} does not expose any explicit `[ref: ...]` plan items yet. "
                "Add refs to the file-backed plan section before binding a task to one."
            )
    return resolved_plan_id, resolved_revision_id, resolved_plan_item_ref

def create_plan(
    ctx: ServerContext,
    repo_name: str,
    title: str,
    artifact_path: str,
    artifact_selector: str | None,
    artifact_heading: str,
    items: list[dict[str, Any]],
    *,
    summary: str | None = None,
    status: str = "draft",
    plan_id: str | None = None,
    source_kind: str = "manual_edit",
    source_session_id: str | None = None,
    artifact_body: str | None = None,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    repo_id = _repo_id(ctx, repo_name)
    repo_namespace_prefix = _repo_id_namespace_prefix(ctx, repo_name)
    normalized_title = _normalize_nonempty_text(title, field="Plan title")
    normalized_status = _normalize_plan_status(status)
    normalized_source_kind = _normalize_nonempty_text(source_kind, field="Plan source_kind")
    normalized_actor_identity = _normalize_actor_identity(actor_identity)
    normalized_actor_type = _normalize_actor_type(actor_type)
    normalized_artifact_path, normalized_artifact_selector, normalized_artifact_heading, normalized_items = _normalize_plan_revision_artifact(
        artifact_path=artifact_path,
        artifact_selector=artifact_selector,
        artifact_heading=artifact_heading,
        items=items,
    )
    plan_links_surface_hash, _ = _plan_link_metadata_for_revision(
        None,
        ctx,
        plan_revision_id="",
        items=normalized_items,
        artifact_body=artifact_body,
    )
    with connect(ctx) as conn:
        if plan_id is None:
            plan_id = generate_namespaced_workflow_id("PL", repo_namespace_prefix)
        existing = conn.execute("select * from plans where plan_id = ?", (plan_id,)).fetchone()
        if existing is not None:
            raise ValueError(f"Plan {plan_id} already exists")

        now = utc_now()
        plan_revision_id = generate_namespaced_workflow_id("PR", repo_namespace_prefix)
        conn.execute(
            """
            insert into plans(
                plan_id, repo_name, repo_id, title, status, head_revision_id, created_by, created_at, updated_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                plan_id,
                repo_name,
                repo_id,
                normalized_title,
                normalized_status,
                plan_revision_id,
                normalized_actor_identity,
                now,
                now,
            ),
        )
        conn.execute(
            """
            insert into plan_revisions(
                plan_revision_id, plan_id, revision_number, parent_plan_revision_id, title_snapshot, summary,
                artifact_path, artifact_selector, artifact_heading, items_json,
                plan_links_surface_hash, plan_links_changed_count_to_prev,
                source_kind, source_session_id, created_by, actor_type, created_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                plan_revision_id,
                plan_id,
                1,
                None,
                normalized_title,
                _normalize_optional_text(summary),
                normalized_artifact_path,
                normalized_artifact_selector,
                normalized_artifact_heading,
                json.dumps(normalized_items, sort_keys=True),
                plan_links_surface_hash,
                0,
                normalized_source_kind,
                _normalize_optional_text(source_session_id),
                normalized_actor_identity,
                normalized_actor_type,
                now,
            ),
        )
        artifact_blob_id = _store_plan_revision_blob(
            conn,
            ctx,
            repo_name=repo_name,
            repo_id=repo_id,
            plan_revision_id=plan_revision_id,
            artifact_body=artifact_body,
            created_at=now,
        )
        record_event(
            conn,
            "plan.created",
            "plan",
            plan_id,
            {
                "repo_name": repo_name,
                "plan_revision_id": plan_revision_id,
                "revision_number": 1,
                "title": normalized_title,
                "status": normalized_status,
                "artifact_blob_id": artifact_blob_id,
            },
            actor_identity=normalized_actor_identity,
            actor_type=normalized_actor_type,
        )
        conn.commit()
        row = conn.execute("select * from plans where plan_id = ?", (plan_id,)).fetchone()
        out = _plan_view(conn, row)
    assert out is not None
    return out

def list_plans(
    ctx: ServerContext,
    repo_name: str,
    *,
    artifact_path: str | None = None,
) -> list[dict]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    repo_id = _repo_id(ctx, repo_name)
    normalized_artifact_path = _normalize_optional_text(artifact_path)
    where_clauses = ["(p.repo_id = ? or (p.repo_id is null and p.repo_name = ?))"]
    params: list[Any] = [repo_id, repo_name]
    if normalized_artifact_path is not None:
        where_clauses.append("pr.artifact_path = ?")
        params.append(normalized_artifact_path)
    with connect(ctx) as conn:
        rows = [
            dict(r)
            for r in conn.execute(
                f"""
                select
                    p.*,
                    pr.revision_number as head_revision_number,
                    pr.artifact_selector as head_artifact_selector,
                    pr.artifact_path as head_artifact_path,
                    pr.artifact_heading as head_artifact_heading,
                    prb.blob_id as head_artifact_blob_id,
                    pr.items_json as head_revision_items_json,
                    pr.summary as head_revision_summary,
                    pr.created_at as head_revision_created_at
                from plans p
                left join plan_revisions pr on pr.plan_revision_id = p.head_revision_id
                left join plan_revision_blobs prb on prb.plan_revision_id = p.head_revision_id
                where {" and ".join(where_clauses)}
                order by p.updated_at desc, p.created_at desc
                """,
                tuple(params),
            )
        ]
    return rows

def get_plan(ctx: ServerContext, plan_id: str) -> dict:
    with connect(ctx) as conn:
        row = _get_plan_row(conn, plan_id)
        out = _plan_view(conn, row)
    assert out is not None
    return out

def list_plan_revisions(ctx: ServerContext, plan_id: str) -> list[dict]:
    with connect(ctx) as conn:
        _get_plan_row(conn, plan_id)
        _ensure_plan_link_diff_metadata(conn, ctx, plan_id=plan_id)
        rows = [
            r
            for r in conn.execute(
                """
                select
                    plan_revision_id, plan_id, revision_number, parent_plan_revision_id, title_snapshot,
                    summary, artifact_path, artifact_selector, artifact_heading, items_json,
                    plan_links_surface_hash, plan_links_changed_count_to_prev,
                    source_kind, source_session_id, created_by, actor_type, created_at
                from plan_revisions
                where plan_id = ?
                order by revision_number desc
                """,
                (plan_id,),
            )
        ]
        out = [_plan_revision_view(row, conn) or {} for row in rows]
    return out

def get_plan_revision(ctx: ServerContext, plan_id: str, plan_revision_id: str) -> dict:
    with connect(ctx) as conn:
        _get_plan_row(conn, plan_id)
        _ensure_plan_link_diff_metadata(conn, ctx, plan_id=plan_id)
        row = conn.execute(
            "select * from plan_revisions where plan_id = ? and plan_revision_id = ?",
            (plan_id, plan_revision_id),
        ).fetchone()
        if row is None:
            raise KeyError(f"Unknown plan revision: {plan_revision_id}")
        out = _plan_revision_view(row, conn, ctx=ctx, include_artifact_body=True) or {}
    return out

def revise_plan(
    ctx: ServerContext,
    plan_id: str,
    artifact_path: str,
    artifact_selector: str | None,
    artifact_heading: str,
    items: list[dict[str, Any]],
    *,
    title: str | None = None,
    summary: str | None = None,
    source_kind: str = "manual_edit",
    source_session_id: str | None = None,
    artifact_body: str | None = None,
    expected_head_revision_id: str | None = None,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict:
    normalized_source_kind = _normalize_nonempty_text(source_kind, field="Plan source_kind")
    normalized_actor_identity = _normalize_actor_identity(actor_identity)
    normalized_actor_type = _normalize_actor_type(actor_type)
    normalized_artifact_path, normalized_artifact_selector, normalized_artifact_heading, normalized_items = _normalize_plan_revision_artifact(
        artifact_path=artifact_path,
        artifact_selector=artifact_selector,
        artifact_heading=artifact_heading,
        items=items,
    )
    with connect(ctx) as conn:
        plan_row = _get_plan_row(conn, plan_id)
        repo_namespace_prefix = _repo_id_namespace_prefix(ctx, plan_row["repo_name"])
        plan_repo_id = _normalize_optional_text(plan_row.get("repo_id")) or _repo_id(ctx, plan_row["repo_name"])
        current_title = _normalize_nonempty_text(title or plan_row["title"], field="Plan title")
        head_revision_id = plan_row["head_revision_id"]
        normalized_expected_head = _normalize_optional_text(expected_head_revision_id)
        if normalized_expected_head is not None and head_revision_id != normalized_expected_head:
            raise ValueError(f"Plan {plan_id} head advanced: expected {normalized_expected_head}, got {head_revision_id}")
        next_revision_number = 1
        if head_revision_id:
            head_row = conn.execute(
                "select revision_number, items_json from plan_revisions where plan_revision_id = ?",
                (head_revision_id,),
            ).fetchone()
            if head_row is not None:
                next_revision_number = int(head_row["revision_number"]) + 1
                try:
                    previous_items = json.loads(head_row["items_json"] or "[]")
                except Exception:
                    previous_items = []
            else:
                previous_items = []
        else:
            previous_items = []
        new_revision_id = generate_namespaced_workflow_id("PR", repo_namespace_prefix)
        now = utc_now()
        previous_entries: dict[str, dict[str, Any]] | None = None
        if head_revision_id:
            _, previous_entries = _plan_link_metadata_for_revision(
                conn,
                ctx,
                plan_revision_id=head_revision_id,
                items=previous_items,
            )
        plan_links_surface_hash, current_entries = _plan_link_metadata_for_revision(
            conn,
            ctx,
            plan_revision_id=new_revision_id,
            items=normalized_items,
            artifact_body=artifact_body,
        )
        changed_count = _plan_link_changed_count(previous_entries, current_entries)
        conn.execute(
            """
            insert into plan_revisions(
                plan_revision_id, plan_id, revision_number, parent_plan_revision_id, title_snapshot, summary,
                artifact_path, artifact_selector, artifact_heading, items_json,
                plan_links_surface_hash, plan_links_changed_count_to_prev,
                source_kind, source_session_id, created_by, actor_type, created_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                new_revision_id,
                plan_id,
                next_revision_number,
                head_revision_id,
                current_title,
                _normalize_optional_text(summary),
                normalized_artifact_path,
                normalized_artifact_selector,
                normalized_artifact_heading,
                json.dumps(normalized_items, sort_keys=True),
                plan_links_surface_hash,
                changed_count,
                normalized_source_kind,
                _normalize_optional_text(source_session_id),
                normalized_actor_identity,
                normalized_actor_type,
                now,
            ),
        )
        conn.execute(
            "update plans set title = ?, head_revision_id = ?, updated_at = ? where plan_id = ?",
            (current_title, new_revision_id, now, plan_id),
        )
        artifact_blob_id = _store_plan_revision_blob(
            conn,
            ctx,
            repo_name=plan_row["repo_name"],
            repo_id=plan_repo_id,
            plan_revision_id=new_revision_id,
            artifact_body=artifact_body,
            created_at=now,
        )
        record_event(
            conn,
            "plan.revised",
            "plan",
            plan_id,
            {
                "repo_name": plan_row["repo_name"],
                "plan_revision_id": new_revision_id,
                "parent_plan_revision_id": head_revision_id,
                "revision_number": next_revision_number,
                "title": current_title,
                "artifact_blob_id": artifact_blob_id,
            },
            actor_identity=normalized_actor_identity,
            actor_type=normalized_actor_type,
        )
        conn.commit()
        row = conn.execute("select * from plans where plan_id = ?", (plan_id,)).fetchone()
        out = _plan_view(conn, row)
    assert out is not None
    return out

def update_plan_status(
    ctx: ServerContext,
    plan_id: str,
    status: str,
    *,
    actor_identity: str | None = None,
    actor_type: str | None = None,
) -> dict:
    normalized_status = _normalize_plan_status(status)
    normalized_actor_identity = _normalize_actor_identity(actor_identity)
    normalized_actor_type = _normalize_actor_type(actor_type)
    with connect(ctx) as conn:
        plan_row = _get_plan_row(conn, plan_id)
        if plan_row["status"] == normalized_status:
            out = _plan_view(conn, plan_row)
            assert out is not None
            return out
        now = utc_now()
        conn.execute("update plans set status = ?, updated_at = ? where plan_id = ?", (normalized_status, now, plan_id))
        record_event(
            conn,
            "plan.status_updated",
            "plan",
            plan_id,
            {
                "repo_name": plan_row["repo_name"],
                "previous_status": plan_row["status"],
                "status": normalized_status,
            },
            actor_identity=normalized_actor_identity,
            actor_type=normalized_actor_type,
        )
        conn.commit()
        row = conn.execute("select * from plans where plan_id = ?", (plan_id,)).fetchone()
        out = _plan_view(conn, row)
    assert out is not None
    return out
