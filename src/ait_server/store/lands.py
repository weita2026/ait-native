from __future__ import annotations

import json
import time
from typing import Any, Mapping

from ait_protocol.common import utc_now

from ..server_content import (
    archive_line as archive_content_line,
    connect as connect_content,
    get_line as get_content_line,
    get_repository as get_content_repository,
    list_lines_by_head_snapshot_ids as list_content_lines_by_head_snapshot_ids,
    read_ref as read_content_ref,
    update_line as update_content_line,
)
from ..server_control import connect, record_event
from ..server_paths import ServerContext
from .land_request_payloads import (
    elapsed_ms as _elapsed_ms,
    land_freshness_result as _land_freshness_result,
    land_request_payload as _land_request_payload,
    phase_timings_from_result as _phase_timings_from_result,
)
from .land_validation import _validate_task_graph_change_land_request
from .repo_ops import _repo_id
from .workflow_artifacts import _land_submission_id_for_change


def _legacy_server_store_module():
    from .. import server_store as legacy_server_store

    return legacy_server_store


def _assert_repo_scope(*args, **kwargs):
    return _legacy_server_store_module()._assert_repo_scope(*args, **kwargs)


def _ensure_change_mutable(*args, **kwargs):
    return _legacy_server_store_module()._ensure_change_mutable(*args, **kwargs)


def _next_repo_sequence(*args, **kwargs):
    return _legacy_server_store_module()._next_repo_sequence(*args, **kwargs)


def _refresh_change_state(*args, **kwargs):
    return _legacy_server_store_module()._refresh_change_state(*args, **kwargs)


def _refresh_stacks_for_change(*args, **kwargs):
    return _legacy_server_store_module()._refresh_stacks_for_change(*args, **kwargs)


def _repo_name_for_repo_id(*args, **kwargs):
    return _legacy_server_store_module()._repo_name_for_repo_id(*args, **kwargs)


def _repo_scope_predicate(*args, **kwargs):
    return _legacy_server_store_module()._repo_scope_predicate(*args, **kwargs)


def _repo_scoped_sequence_ref(*args, **kwargs):
    return _legacy_server_store_module()._repo_scoped_sequence_ref(*args, **kwargs)


def _resolve_patchset_for_change(*args, **kwargs):
    return _legacy_server_store_module()._resolve_patchset_for_change(*args, **kwargs)


def evaluate_policy(*args, **kwargs):
    return _legacy_server_store_module().evaluate_policy(*args, **kwargs)


def get_change_for_repo(*args, **kwargs):
    return _legacy_server_store_module().get_change_for_repo(*args, **kwargs)


def get_patchset_for_repo(*args, **kwargs):
    return _legacy_server_store_module().get_patchset_for_repo(*args, **kwargs)


def _snapshot_root_tree_id(ctx: ServerContext, snapshot_id: str | None, *, conn=None) -> str | None:
    normalized = str(snapshot_id or "").strip()
    if not normalized:
        return None
    if conn is None:
        with connect_content(ctx) as content_conn:
            return _snapshot_root_tree_id(ctx, normalized, conn=content_conn)
    row = conn.execute("select root_tree_id from snapshots where snapshot_id = ?", (normalized,)).fetchone()
    if row is None:
        return None
    root_tree_id = str(row["root_tree_id"] or "").strip()
    return root_tree_id or None


def _land_snapshot_alignment(
    ctx: ServerContext,
    *,
    target_line_head: str | None,
    revision_snapshot_id: str | None,
    conn=None,
) -> dict[str, Any]:
    normalized_target = str(target_line_head or "").strip() or None
    normalized_revision = str(revision_snapshot_id or "").strip() or None
    target_matches_revision_snapshot = bool(
        normalized_target and normalized_revision and normalized_target == normalized_revision
    )
    target_root_tree_id: str | None = None
    revision_root_tree_id: str | None = None
    if not target_matches_revision_snapshot:
        target_root_tree_id = _snapshot_root_tree_id(ctx, normalized_target, conn=conn)
        revision_root_tree_id = _snapshot_root_tree_id(ctx, normalized_revision, conn=conn)
    target_matches_revision_tree = bool(
        normalized_target
        and normalized_revision
        and (
            target_matches_revision_snapshot
            or (
                target_root_tree_id
                and revision_root_tree_id
                and target_root_tree_id == revision_root_tree_id
            )
        )
    )
    return {
        "target_line_head": normalized_target,
        "revision_snapshot_id": normalized_revision,
        "target_root_tree_id": target_root_tree_id,
        "revision_root_tree_id": revision_root_tree_id,
        "target_matches_revision_snapshot": target_matches_revision_snapshot,
        "target_matches_revision_tree": target_matches_revision_tree,
        "already_aligned_equivalent": target_matches_revision_tree,
    }


def _land_request_context(conn, submission_id: str) -> tuple[dict, dict]:
    land = conn.execute("select * from land_requests where submission_id = ?", (submission_id,)).fetchone()
    if land is None:
        raise KeyError(f"Unknown land request: {submission_id}")
    change = conn.execute("select * from changes where change_id = ?", (land["change_id"],)).fetchone()
    if change is None:
        raise KeyError(f"Unknown change: {land['change_id']}")
    return dict(land), dict(change)


def _target_line_running_request(conn, repo_id: str, repo_name: str, target_line: str) -> dict | None:
    row = conn.execute(
        """
        select lr.*
        from land_requests lr
        join changes c on c.change_id = lr.change_id
        where """
        + _repo_scope_predicate(alias="c")
        + """ and lr.target_line = ? and lr.status = 'running'
        order by lr.updated_at asc, lr.submission_id asc
        limit 1
        """,
        (repo_id, repo_name, target_line),
    ).fetchone()
    return dict(row) if row is not None else None


def _target_line_next_queued_request(conn, repo_id: str, repo_name: str, target_line: str) -> dict | None:
    row = conn.execute(
        """
        select lr.*
        from land_requests lr
        join changes c on c.change_id = lr.change_id
        where """
        + _repo_scope_predicate(alias="c")
        + """ and lr.target_line = ? and lr.status = 'queued'
        order by lr.created_at asc, lr.submission_id asc
        limit 1
        """,
        (repo_id, repo_name, target_line),
    ).fetchone()
    return dict(row) if row is not None else None


def _land_freshness_preflight(
    ctx: ServerContext,
    repo_name: str,
    target_line: str,
    patchset: Mapping[str, Any],
    *,
    target_line_head: str | None = None,
) -> dict[str, Any]:
    normalized_target_head = target_line_head
    if normalized_target_head is None:
        normalized_target_head = read_content_ref(ctx, repo_name, target_line)
    expected_base_snapshot_id = str(patchset.get("base_snapshot_id") or "").strip() or None
    revision_snapshot_id = str(patchset.get("revision_snapshot_id") or "").strip() or None
    with connect_content(ctx) as conn:
        alignment = _land_snapshot_alignment(
            ctx,
            target_line_head=normalized_target_head,
            revision_snapshot_id=revision_snapshot_id,
            conn=conn,
        )
    return {
        **_land_freshness_result(
            target_line,
            patchset,
            target_line_head=normalized_target_head,
            alignment=alignment,
        ),
        "expected_base_snapshot_id": expected_base_snapshot_id,
    }


def _snapshot_source_line_name(ctx: ServerContext, repo_name: str, snapshot_id: str | None) -> str | None:
    normalized_snapshot_id = str(snapshot_id or "").strip()
    if not normalized_snapshot_id:
        return None
    with connect_content(ctx) as conn:
        row = conn.execute(
            "select line_name from snapshots where snapshot_id = ? and repo_name = ?",
            (normalized_snapshot_id, repo_name),
        ).fetchone()
    if row is None:
        return None
    return str(row["line_name"] or "").strip() or None


def _record_landed_line_archive_event(
    conn,
    *,
    repo_name: str,
    line_name: str,
    archived: Mapping[str, Any],
    target_line: str,
    landed_snapshot_id: str,
    change_id: str,
) -> None:
    record_event(
        conn,
        "line.archived",
        "line",
        f"{repo_name}:{line_name}",
        {
            "repo_name": repo_name,
            "line_name": line_name,
            "status": archived["status"],
            "archived_at": archived.get("archived_at"),
            "reason": "landed_snapshot_promoted",
            "target_line": target_line,
            "landed_snapshot_id": landed_snapshot_id,
            "change_id": change_id,
        },
    )


def _archive_landed_line_candidate(
    ctx: ServerContext,
    conn,
    *,
    repo_name: str,
    line_name: str,
    default_line: str,
    target_line: str,
    eligible_snapshot_ids: set[str],
    landed_snapshot_id: str,
    change_id: str,
) -> tuple[str | None, str]:
    if line_name == target_line or line_name == default_line:
        return None, "protected"
    try:
        line = get_content_line(ctx, repo_name, line_name)
    except KeyError:
        return None, "missing"
    if (line.get("status") or "active") == "archived":
        return None, "archived"
    if line.get("head_snapshot_id") not in eligible_snapshot_ids:
        return None, "head_mismatch"
    archived = archive_content_line(ctx, repo_name, line_name)
    _record_landed_line_archive_event(
        conn,
        repo_name=repo_name,
        line_name=line_name,
        archived=archived,
        target_line=target_line,
        landed_snapshot_id=landed_snapshot_id,
        change_id=change_id,
    )
    return archived["line_name"], "archived_now"


def _archive_landed_source_lines(
    ctx: ServerContext,
    conn,
    *,
    repo_name: str,
    revision_snapshot_id: str | None,
    landed_snapshot_id: str,
    default_line: str,
    target_line: str,
    change_id: str,
) -> tuple[list[str], dict[str, Any]]:
    total_started = time.perf_counter()
    eligible_snapshot_ids = {
        value
        for value in {
            str(landed_snapshot_id or "").strip() or None,
            str(revision_snapshot_id or "").strip() or None,
        }
        if value
    }
    source_line_name = _snapshot_source_line_name(ctx, repo_name, revision_snapshot_id)
    archived_lines: list[str] = []
    direct_lookup_state = "not_attempted"
    direct_started = time.perf_counter()
    if source_line_name is not None:
        archived, direct_lookup_state = _archive_landed_line_candidate(
            ctx,
            conn,
            repo_name=repo_name,
            line_name=source_line_name,
            default_line=default_line,
            target_line=target_line,
            eligible_snapshot_ids=eligible_snapshot_ids,
            landed_snapshot_id=landed_snapshot_id,
            change_id=change_id,
        )
        if archived is not None:
            archived_lines.append(archived)
    direct_ms = _elapsed_ms(direct_started)

    fallback_scan_used = not archived_lines
    fallback_scan_ms = 0.0
    fallback_candidates = []
    if fallback_scan_used and source_line_name is not None and direct_lookup_state in {"missing", "archived"}:
        fallback_scan_used = False
    elif fallback_scan_used:
        scan_started = time.perf_counter()
        fallback_candidates = list_content_lines_by_head_snapshot_ids(
            ctx,
            repo_name,
            eligible_snapshot_ids,
            exclude_line_names={default_line, target_line},
        )
        for line in fallback_candidates:
            archived, _ = _archive_landed_line_candidate(
                ctx,
                conn,
                repo_name=repo_name,
                line_name=line["line_name"],
                default_line=default_line,
                target_line=target_line,
                eligible_snapshot_ids=eligible_snapshot_ids,
                landed_snapshot_id=landed_snapshot_id,
                change_id=change_id,
            )
            if archived is not None:
                archived_lines.append(archived)
        fallback_scan_ms = _elapsed_ms(scan_started)

    strategy = "direct_source_line"
    if fallback_scan_used:
        strategy = "indexed_head_lookup"
    if not archived_lines:
        if source_line_name is not None and direct_lookup_state in {"missing", "archived"}:
            strategy = "known_source_unavailable"
    return archived_lines, {
        "strategy": strategy,
        "source_line_name": source_line_name,
        "source_line_state": direct_lookup_state,
        "direct_lookup": direct_ms,
        "fallback_scan_used": fallback_scan_used,
        "fallback_scan": fallback_scan_ms,
        "fallback_candidate_count": len(fallback_candidates),
        "total": _elapsed_ms(total_started),
    }


def create_land_request(ctx: ServerContext, change_id: str, patchset_id: str | None, target_line: str, mode: str) -> dict:
    request_started = time.perf_counter()
    with connect(ctx) as conn:
        change = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
        if change is None:
            raise KeyError(f"Unknown change: {change_id}")
        _ensure_change_mutable(change, "submit land requests")
        _validate_task_graph_change_land_request(conn, dict(change))
        patchset_started = time.perf_counter()
        patchset = _resolve_patchset_for_change(conn, change_id, patchset_id)
        patchset_ms = _elapsed_ms(patchset_started)
        count = conn.execute("select count(*) as c from land_requests where change_id = ?", (change_id,)).fetchone()["c"]
        submission_id = _land_submission_id_for_change(change_id, int(count))
        existing_started = time.perf_counter()
        existing = conn.execute(
            """
            select *
            from land_requests
            where change_id = ?
              and patchset_id = ?
              and target_line = ?
              and mode = ?
              and status in ('queued', 'running', 'succeeded')
            order by created_at desc, submission_id desc
            limit 1
            """,
            (change_id, patchset["patchset_id"], target_line, mode),
        ).fetchone()
        existing_lookup_ms = _elapsed_ms(existing_started)
        if existing is not None:
            return _land_request_payload(existing)
        repo_id = str(change["repo_id"] or "").strip() or _repo_id(ctx, change["repo_name"])
        repo_name = _repo_name_for_repo_id(ctx, repo_id, change["repo_name"])
        preflight_started = time.perf_counter()
        target_line_row = get_content_line(ctx, repo_name, target_line)
        freshness_preflight = _land_freshness_preflight(
            ctx,
            repo_name,
            target_line,
            patchset,
            target_line_head=target_line_row.get("head_snapshot_id"),
        )
        preflight_ms = _elapsed_ms(preflight_started)
        land_seq = _next_repo_sequence(conn, "land_requests", repo_id, "land_seq")
        now = utc_now()
        result_payload = {
            "freshness_preflight": freshness_preflight,
            "phase_timings_ms": {
                "create_land_request": {
                    "resolve_patchset": patchset_ms,
                    "existing_request_lookup": existing_lookup_ms,
                    "target_line_preflight": preflight_ms,
                    "total": _elapsed_ms(request_started),
                }
            },
        }
        conn.execute(
            "insert into land_requests(submission_id, repo_id, land_seq, change_id, patchset_id, target_line, mode, status, result_json, created_at, updated_at) values (?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?)",
            (
                submission_id,
                repo_id,
                land_seq,
                change_id,
                patchset["patchset_id"],
                target_line,
                mode,
                json.dumps(result_payload, sort_keys=True),
                now,
                now,
            ),
        )
        record_event(
            conn,
            "land.requested",
            "change",
            change_id,
            {"submission_id": submission_id, "patchset_id": patchset["patchset_id"], "target_line": target_line, "mode": mode},
        )
        conn.commit()
        row = conn.execute("select * from land_requests where submission_id = ?", (submission_id,)).fetchone()
    return _land_request_payload(row)


def submit_land(ctx: ServerContext, change_id: str, patchset_id: str | None, target_line: str, mode: str, *, inline: bool = True) -> dict:
    land = create_land_request(ctx, change_id, patchset_id, target_line, mode)
    if inline:
        freshness_preflight = (
            (land.get("result") or {}).get("freshness_preflight")
            if isinstance(land.get("result"), Mapping)
            else None
        )
        if (
            isinstance(freshness_preflight, Mapping)
            and not bool(freshness_preflight.get("base_is_fresh"))
            and not bool(freshness_preflight.get("already_aligned_equivalent"))
            and land.get("status") == "queued"
        ):
            phase_timings = _phase_timings_from_result(land.get("result"))
            blocked_result = {
                "blocker_class": "BASE_STALE",
                "target_line_head": freshness_preflight.get("target_line_head"),
                "expected_base_snapshot_id": freshness_preflight.get("expected_base_snapshot_id"),
                "freshness_preflight": dict(freshness_preflight),
            }
            if phase_timings:
                blocked_result["phase_timings_ms"] = phase_timings
            now = utc_now()
            with connect(ctx) as conn:
                conn.execute(
                    "update land_requests set status = 'blocked', result_json = ?, updated_at = ? where submission_id = ?",
                    (json.dumps(blocked_result, sort_keys=True), now, land["submission_id"]),
                )
                record_event(
                    conn,
                    "land.blocked",
                    "change",
                    change_id,
                    {"submission_id": land["submission_id"], "reason": "BASE_STALE"},
                )
                conn.commit()
                row = conn.execute("select * from land_requests where submission_id = ?", (land["submission_id"],)).fetchone()
            return _land_request_payload(row)
        return _process_land(ctx, land["submission_id"])
    return land


def submit_land_for_repo(
    ctx: ServerContext,
    repo_name: str,
    change_ref: str,
    patchset_ref: str | None,
    target_line: str,
    mode: str,
    *,
    inline: bool = True,
) -> dict:
    change = get_change_for_repo(ctx, repo_name, change_ref)
    patchset_id = None
    if patchset_ref is not None:
        patchset = get_patchset_for_repo(ctx, repo_name, patchset_ref, change_ref=change_ref)
        patchset_id = patchset["patchset_id"]
    return submit_land(ctx, change["change_id"], patchset_id, target_line, mode, inline=inline)


def _process_land(ctx: ServerContext, submission_id: str) -> dict:
    with connect(ctx) as conn:
        requested, change = _land_request_context(conn, submission_id)
    repo_name = change["repo_name"]
    repo_id = str(change.get("repo_id") or "").strip() or _repo_id(ctx, repo_name)
    target_line = requested["target_line"]

    while True:
        with connect(ctx) as conn:
            requested, _ = _land_request_context(conn, submission_id)
            if requested["status"] != "queued":
                return _land_request_payload(requested)
            running = _target_line_running_request(conn, repo_id, repo_name, target_line)
            if running is not None:
                return _land_request_payload(requested)
            next_land = _target_line_next_queued_request(conn, repo_id, repo_name, target_line)
            if next_land is None:
                return _land_request_payload(requested)

        processed = _process_single_land(ctx, next_land["submission_id"])
        if next_land["submission_id"] == submission_id:
            return processed


def _process_single_land(ctx: ServerContext, submission_id: str) -> dict:
    process_started = time.perf_counter()
    with connect(ctx) as conn:
        land = conn.execute("select * from land_requests where submission_id = ?", (submission_id,)).fetchone()
        if land is None:
            raise KeyError(f"Unknown land request: {submission_id}")
        change = conn.execute("select * from changes where change_id = ?", (land["change_id"],)).fetchone()
        patchset = conn.execute("select * from patchsets where patchset_id = ?", (land["patchset_id"],)).fetchone()
        assert change is not None and patchset is not None
        if land["status"] != "queued":
            return _land_request_payload(land)
        conn.execute("update land_requests set status = 'running', updated_at = ? where submission_id = ?", (utc_now(), submission_id))
        record_event(conn, "land.started", "change", change["change_id"], {"submission_id": submission_id})
        conn.commit()

    policy_started = time.perf_counter()
    policy = evaluate_policy(ctx, patchset["patchset_id"])
    policy_ms = _elapsed_ms(policy_started)

    with connect(ctx) as conn:
        current_land = conn.execute("select * from land_requests where submission_id = ?", (submission_id,)).fetchone()
        change = conn.execute("select * from changes where change_id = ?", (current_land["change_id"],)).fetchone()
        patchset = conn.execute("select * from patchsets where patchset_id = ?", (current_land["patchset_id"],)).fetchone()
        assert change is not None and patchset is not None
        repo_id = str(change["repo_id"] or "").strip() or _repo_id(ctx, change["repo_name"])
        repo_name = _repo_name_for_repo_id(ctx, repo_id, change["repo_name"])
        existing_result = json.loads(current_land["result_json"] or "{}")
        freshness_preflight = existing_result.get("freshness_preflight") if isinstance(existing_result, Mapping) else None
        phase_timings = _phase_timings_from_result(existing_result)
        phase_timings["policy_evaluation"] = policy_ms

        target_head_started = time.perf_counter()
        target_line_head = read_content_ref(ctx, repo_name, current_land["target_line"])
        phase_timings["target_line_head_read"] = _elapsed_ms(target_head_started)
        alignment_started = time.perf_counter()
        with connect_content(ctx) as content_conn:
            alignment = _land_snapshot_alignment(
                ctx,
                target_line_head=target_line_head,
                revision_snapshot_id=patchset["revision_snapshot_id"],
                conn=content_conn,
            )
        phase_timings["target_alignment"] = _elapsed_ms(alignment_started)
        result: dict[str, Any]
        status: str
        if policy["decision"] != "pass":
            status = "blocked"
            result = {"blocker_class": "POLICY_BLOCKED", "policy": policy}
            record_event(
                conn,
                "land.blocked",
                "change",
                change["change_id"],
                {"submission_id": submission_id, "reason": "POLICY_BLOCKED"},
            )
        elif bool(alignment.get("already_aligned_equivalent")):
            status = "succeeded"
            landed_at = utc_now()
            landed_snapshot_id = str(target_line_head or patchset["revision_snapshot_id"])
            result = {
                "landed_snapshot_id": landed_snapshot_id,
                "selected_revision_snapshot_id": patchset["revision_snapshot_id"],
                "target_line": current_land["target_line"],
                "base_snapshot_id": patchset["base_snapshot_id"],
                "line_action": "already_aligned",
                "snapshot_action": (
                    "selected_patchset_revision"
                    if target_line_head == patchset["revision_snapshot_id"]
                    else "reused_equivalent_existing_snapshot"
                ),
            }
            conn.execute(
                "update changes set status = 'landed', selected_patchset_number = ?, landed_at = ?, updated_at = ? where change_id = ?",
                (patchset["patchset_number"], landed_at, landed_at, change["change_id"]),
            )
            default_line = get_content_repository(ctx, repo_name)["default_line"]
            archived_lines, archive_timings = _archive_landed_source_lines(
                ctx,
                conn,
                repo_name=repo_name,
                revision_snapshot_id=patchset["revision_snapshot_id"],
                landed_snapshot_id=landed_snapshot_id,
                default_line=default_line,
                target_line=current_land["target_line"],
                change_id=change["change_id"],
            )
            phase_timings["archive_lines"] = archive_timings
            if archived_lines:
                result["archived_lines"] = archived_lines
            other_changes = [
                row["change_id"]
                for row in conn.execute(
                    "select change_id from changes where " + _repo_scope_predicate() + " and base_line = ? and change_id != ? and status not in ('landed','archived')",
                    (repo_id, repo_name, current_land["target_line"], change["change_id"]),
                ).fetchall()
            ]
            for other_change_id in other_changes:
                _refresh_change_state(ctx, conn, other_change_id)
            _refresh_stacks_for_change(conn, change["change_id"])
            record_event(
                conn,
                "change.landed",
                "change",
                change["change_id"],
                {
                    "submission_id": submission_id,
                    "patchset_id": patchset["patchset_id"],
                    "target_line": current_land["target_line"],
                    "line_action": "already_aligned",
                    "landed_snapshot_id": landed_snapshot_id,
                    "selected_revision_snapshot_id": patchset["revision_snapshot_id"],
                },
            )
        elif target_line_head != patchset["base_snapshot_id"]:
            status = "blocked"
            result = {
                "blocker_class": "BASE_STALE",
                "target_line_head": target_line_head,
                "expected_base_snapshot_id": patchset["base_snapshot_id"],
            }
            record_event(
                conn,
                "land.blocked",
                "change",
                change["change_id"],
                {"submission_id": submission_id, "reason": "BASE_STALE"},
            )
        else:
            status = "succeeded"
            landed_at = utc_now()
            result = {
                "landed_snapshot_id": patchset["revision_snapshot_id"],
                "selected_revision_snapshot_id": patchset["revision_snapshot_id"],
                "target_line": current_land["target_line"],
                "base_snapshot_id": patchset["base_snapshot_id"],
                "line_action": "moved",
                "snapshot_action": "selected_patchset_revision",
            }
            update_timings: dict[str, Any] = {}
            update_content_line(
                ctx,
                repo_name,
                current_land["target_line"],
                patchset["revision_snapshot_id"],
                expected_head_snapshot_id=target_line_head,
                timings=update_timings,
            )
            phase_timings["target_line_update"] = update_timings
            conn.execute(
                "update changes set status = 'landed', selected_patchset_number = ?, landed_at = ?, updated_at = ? where change_id = ?",
                (patchset["patchset_number"], landed_at, landed_at, change["change_id"]),
            )
            default_line = get_content_repository(ctx, repo_name)["default_line"]
            archived_lines, archive_timings = _archive_landed_source_lines(
                ctx,
                conn,
                repo_name=repo_name,
                revision_snapshot_id=patchset["revision_snapshot_id"],
                landed_snapshot_id=patchset["revision_snapshot_id"],
                default_line=default_line,
                target_line=current_land["target_line"],
                change_id=change["change_id"],
            )
            phase_timings["archive_lines"] = archive_timings
            if archived_lines:
                result["archived_lines"] = archived_lines
            other_changes = [
                row["change_id"]
                for row in conn.execute(
                    "select change_id from changes where " + _repo_scope_predicate() + " and base_line = ? and change_id != ? and status not in ('landed','archived')",
                    (repo_id, repo_name, current_land["target_line"], change["change_id"]),
                ).fetchall()
            ]
            for other_change_id in other_changes:
                _refresh_change_state(ctx, conn, other_change_id)
            _refresh_stacks_for_change(conn, change["change_id"])
            record_event(
                conn,
                "change.landed",
                "change",
                change["change_id"],
                {"submission_id": submission_id, "patchset_id": patchset["patchset_id"], "target_line": current_land["target_line"]},
            )

        if isinstance(freshness_preflight, Mapping):
            refreshed_head = target_line_head
            refreshed_alignment: Mapping[str, Any] | None = alignment
            if status == "succeeded":
                refreshed_head = str(result.get("landed_snapshot_id") or "").strip() or target_line_head
                if result.get("line_action") == "moved":
                    refreshed_alignment = {
                        "target_matches_revision_snapshot": True,
                        "target_matches_revision_tree": True,
                        "already_aligned_equivalent": True,
                    }
            result["freshness_preflight"] = {
                **dict(freshness_preflight),
                **_land_freshness_result(
                    current_land["target_line"],
                    patchset,
                    target_line_head=refreshed_head,
                    alignment=refreshed_alignment,
                ),
            }

        phase_timings["total_process_land"] = _elapsed_ms(process_started)
        result["phase_timings_ms"] = phase_timings
        now = utc_now()
        conn.execute(
            "update land_requests set status = ?, result_json = ?, updated_at = ? where submission_id = ?",
            (status, json.dumps(result, sort_keys=True), now, submission_id),
        )
        conn.commit()
        row = conn.execute("select * from land_requests where submission_id = ?", (submission_id,)).fetchone()
    return _land_request_payload(row)


def get_land_request(ctx: ServerContext, submission_id: str) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select * from land_requests where submission_id = ?", (submission_id,)).fetchone()
    if row is None:
        raise KeyError(f"Unknown land request: {submission_id}")
    return _land_request_payload(row)


def get_land_request_for_repo(ctx: ServerContext, repo_name: str, submission_ref: str) -> dict:
    repo_id = _assert_repo_scope(ctx, repo_name)
    with connect(ctx) as conn:
        row = conn.execute(
            "select * from land_requests where repo_id = ? and submission_id = ?",
            (repo_id, submission_ref),
        ).fetchone()
        if row is None:
            land_seq = _repo_scoped_sequence_ref(submission_ref)
            if land_seq is not None:
                row = conn.execute(
                    "select * from land_requests where repo_id = ? and land_seq = ?",
                    (repo_id, land_seq),
                ).fetchone()
    if row is None:
        raise KeyError(f"Unknown land request {submission_ref} for repository {repo_name}")
    return _land_request_payload(row)


def retry_land(ctx: ServerContext, submission_id: str, *, inline: bool = True) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select status from land_requests where submission_id = ?", (submission_id,)).fetchone()
        if row is None:
            raise KeyError(f"Unknown land request: {submission_id}")
        if row["status"] == "succeeded":
            raise ValueError(f"Land request {submission_id} already succeeded")
        conn.execute(
            "update land_requests set status = 'queued', result_json = ?, updated_at = ? where submission_id = ?",
            (json.dumps({}, sort_keys=True), utc_now(), submission_id),
        )
        conn.commit()
        row = conn.execute("select * from land_requests where submission_id = ?", (submission_id,)).fetchone()
    if inline:
        return _process_land(ctx, submission_id)
    return _land_request_payload(row)


def evaluate_policy_for_repo(
    ctx: ServerContext,
    repo_name: str,
    patchset_ref: str,
    *,
    change_ref: str | None = None,
    repo_id: str | None = None,
) -> dict:
    _assert_repo_scope(ctx, repo_name, repo_id)
    patchset = get_patchset_for_repo(ctx, repo_name, patchset_ref, change_ref=change_ref)
    return evaluate_policy(ctx, patchset["patchset_id"])


def process_land_for_repo(
    ctx: ServerContext,
    repo_name: str,
    submission_ref: str,
    *,
    repo_id: str | None = None,
) -> dict:
    _assert_repo_scope(ctx, repo_name, repo_id)
    land = get_land_request_for_repo(ctx, repo_name, submission_ref)
    return _process_land(ctx, land["submission_id"])
