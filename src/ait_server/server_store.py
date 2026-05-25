from __future__ import annotations

import hashlib
import json
from typing import Any

from ait_protocol.common import (
    AuthorMode,
    REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX,
    derive_patchset_id,
    derive_policy_author_class,
    derive_policy_content_class,
    find_plan_item_in_items,
    generate_namespaced_sequence_id,
    lane_from_risk,
    normalize_author_mode,
    normalize_plan_items,
    resolve_effective_policy,
    utc_now,
    workflow_origin_namespace_prefix,
)
from ait_protocol.task_statuses import (
    TASK_REMOTE_CLOSE_TARGET_STATUSES,
    TASK_STATUS_COMPLETED,
    is_task_abandoned_status,
    normalize_task_status,
)
from .server_content import (
    _canonical_snapshot_metadata,
    connect as connect_content,
    ensure_repository as ensure_content_repository,
    export_snapshot as export_content_snapshot,
    get_line as get_content_line,
    get_repository as get_content_repository,
    get_snapshot_repo,
    import_snapshot as import_content_snapshot,
    initialize as initialize_content,
    list_lines as list_content_lines,
    repository_storage_stats as get_content_repository_storage,
    read_ref as read_content_ref,
    repository_exists,
    snapshot_exists,
    snapshot_existence as content_snapshot_existence,
    snapshot_manifest_map,
    update_line as update_content_line,
    pack_repository as pack_content_repository,
    gc_repository_content as gc_content_repository,
    repository_storage_signals,
)
from .authority_store import (
    create_authority_child,
    create_authority_node,
    delete_authority_document,
    delete_authority_node,
    ensure_authority_map,
    get_authority_map,
    get_authority_node,
    list_authority_graph,
    list_authority_mutations,
    list_authority_nodes,
    replace_authority_graph,
    reorder_authority_document,
    reorder_authority_node,
    seed_blank_authority_graph,
    update_authority_node,
)
from .server_control import connect, initialize as initialize_control, latest_policy_status, record_event
from .server_paths import ServerContext
from .store.lands import (
    _process_land,
    create_land_request,
    evaluate_policy_for_repo,
    get_land_request,
    get_land_request_for_repo,
    process_land_for_repo,
    retry_land,
    submit_land,
    submit_land_for_repo,
)
from .store.plans import (
    _get_plan_row,
    _normalize_actor_identity,
    _normalize_actor_type,
    _normalize_nonempty_text,
    _normalize_optional_text,
    _plan_link_changed_count,
    _plan_link_metadata_for_revision,
    _plan_revision_has_task_graph_artifact,
    _normalize_plan_revision_artifact,
    _normalize_plan_status,
    _plan_revision_view,
    _plan_view,
    _resolve_task_plan_linkage,
    _store_plan_revision_blob,
    append_planning_session_event,
    close_planning_session,
    create_plan,
    create_planning_session,
    get_plan,
    get_plan_revision,
    get_planning_session,
    join_planning_session,
    list_plan_revisions,
    list_plans,
    list_planning_session_events,
    list_planning_sessions,
    put_plan_revision_artifacts,
    promote_planning_session,
    revise_plan,
    update_plan_status,
)
from .store.repo_ops import (
    _repo_id,
    _repo_id_namespace_prefix,
    close_line,
    ensure_repository,
    export_snapshot,
    gc_repository_storage,
    get_line,
    get_repository,
    get_repository_storage,
    import_snapshot,
    list_lines,
    optimize_repository_storage,
    pack_repository_storage,
    snapshot_existence,
    update_line,
)
from .store.repo_retire import retire_repository
from .store.releases import (
    get_release,
    get_release_for_repo,
    publish_release,
    read_release_artifact,
)
from .store.reviews import (
    _required_approvals,
    _review_summary,
    list_reviews,
    record_review,
    request_review,
)
from .store.workflow_artifacts import (
    CODE_REVIEW_SUMMARY_ACTION,
    POLICY_REQUIREMENT_MAP,
    RULE_LABELS,
    TASK_REVIEW_APPROVE_ACTION,
    TASK_REVIEW_COMMENT_ACTION,
    TASK_REVIEW_DEFER_ACTION,
    TASK_REVIEW_REQUEST_CHANGES_ACTION,
    TEAM_REVIEW_APPROVE_ACTION,
    _attestation_id_for_patchset,
    _ci_rollout_for_patchset,
    _ci_rollout_patchset_suite_checks,
    _ci_rollout_summary_message,
    _effective_policy_status,
    _invalidate_patchset_policy,
    _patchset_changed_paths,
    _patchset_diff_stats,
    _policy_context_for_patchset,
    _policy_status_view,
    _requires_code_review_summary,
    _review_decision_lane,
)

PLAN_STATUSES = {"draft", "active", "superseded", "archived"}
TASK_PLANNING_STATES = {"planned", "unplanned", "explicit_unplanned"}
RELEASE_STATUSES = {"published"}
HISTORICAL_PUBLICATION_MODES = {"records_only", "attach", "rebase", "reconcile"}


def _repo_scope_predicate(*, alias: str | None = None) -> str:
    prefix = f"{alias}." if alias else ""
    return f"({prefix}repo_id = ? or ({prefix}repo_id is null and {prefix}repo_name = ?))"


def _repo_name_for_repo_id(ctx: ServerContext, repo_id: str | None, fallback_repo_name: str) -> str:
    normalized_repo_id = str(repo_id or "").strip()
    if not normalized_repo_id:
        return fallback_repo_name
    with connect_content(ctx) as conn:
        row = conn.execute("select repo_name from repositories where repo_id = ?", (normalized_repo_id,)).fetchone()
    resolved_repo_name = str(row["repo_name"]).strip() if row is not None else ""
    return resolved_repo_name or fallback_repo_name


def initialize(ctx: ServerContext) -> None:
    initialize_content(ctx)
    initialize_control(ctx)
    _backfill_control_plane_repo_ids(ctx)
    _backfill_control_plane_local_keys(ctx)


def _repository_repo_ids(ctx: ServerContext) -> dict[str, str]:
    with connect_content(ctx) as conn:
        rows = conn.execute("select repo_name, repo_id from repositories").fetchall()
    repo_ids: dict[str, str] = {}
    for row in rows:
        repo_name = str(row["repo_name"] or "").strip()
        repo_id = str(row["repo_id"] or "").strip()
        if repo_name and repo_id:
            repo_ids[repo_name] = repo_id
    return repo_ids


def _update_repo_id(conn, table_name: str, key_fields: tuple[str, ...], key_values: tuple[Any, ...], repo_id: str) -> None:
    if not repo_id:
        return
    where = " and ".join(f"{field} = ?" for field in key_fields)
    conn.execute(
        f"update {table_name} set repo_id = ? where repo_id is null and {where}",
        (repo_id, *key_values),
    )


def _backfill_control_plane_repo_ids(ctx: ServerContext) -> None:
    repo_ids = _repository_repo_ids(ctx)
    if not repo_ids:
        return
    with connect(ctx) as conn:
        for row in conn.execute("select task_id, repo_name from tasks where repo_id is null").fetchall():
            _update_repo_id(conn, "tasks", ("task_id",), (row["task_id"],), repo_ids.get(str(row["repo_name"] or "").strip(), ""))
        for row in conn.execute("select plan_id, repo_name from plans where repo_id is null").fetchall():
            _update_repo_id(conn, "plans", ("plan_id",), (row["plan_id"],), repo_ids.get(str(row["repo_name"] or "").strip(), ""))
        for row in conn.execute("select plan_revision_id, repo_name from plan_revision_blobs where repo_id is null").fetchall():
            _update_repo_id(
                conn,
                "plan_revision_blobs",
                ("plan_revision_id",),
                (row["plan_revision_id"],),
                repo_ids.get(str(row["repo_name"] or "").strip(), ""),
            )
        for row in conn.execute("select release_id, repo_name from releases where repo_id is null").fetchall():
            _update_repo_id(conn, "releases", ("release_id",), (row["release_id"],), repo_ids.get(str(row["repo_name"] or "").strip(), ""))
        for row in conn.execute("select change_id, repo_name from changes where repo_id is null").fetchall():
            _update_repo_id(conn, "changes", ("change_id",), (row["change_id"],), repo_ids.get(str(row["repo_name"] or "").strip(), ""))
        for row in conn.execute("select session_id, repo_name from sessions where repo_id is null").fetchall():
            _update_repo_id(conn, "sessions", ("session_id",), (row["session_id"],), repo_ids.get(str(row["repo_name"] or "").strip(), ""))
        for row in conn.execute("select planning_session_id, repo_name from planning_sessions where repo_id is null").fetchall():
            _update_repo_id(
                conn,
                "planning_sessions",
                ("planning_session_id",),
                (row["planning_session_id"],),
                repo_ids.get(str(row["repo_name"] or "").strip(), ""),
            )
        for row in conn.execute("select stack_id, repo_name from stacks where repo_id is null").fetchall():
            _update_repo_id(conn, "stacks", ("stack_id",), (row["stack_id"],), repo_ids.get(str(row["repo_name"] or "").strip(), ""))
        for row in conn.execute("select job_id, repo_name from jobs where repo_id is null").fetchall():
            _update_repo_id(conn, "jobs", ("job_id",), (row["job_id"],), repo_ids.get(str(row["repo_name"] or "").strip(), ""))

        for row in conn.execute(
            """
            select patchsets.patchset_id, changes.repo_id as repo_id
            from patchsets
            join changes using(change_id)
            where patchsets.repo_id is null
            """
        ).fetchall():
            _update_repo_id(conn, "patchsets", ("patchset_id",), (row["patchset_id"],), str(row["repo_id"] or "").strip())
        for row in conn.execute(
            """
            select review_request_id, changes.repo_id as repo_id
            from review_requests
            join changes using(change_id)
            where review_requests.repo_id is null
            """
        ).fetchall():
            _update_repo_id(conn, "review_requests", ("review_request_id",), (row["review_request_id"],), str(row["repo_id"] or "").strip())
        for row in conn.execute(
            """
            select review_id, changes.repo_id as repo_id
            from reviews
            join changes using(change_id)
            where reviews.repo_id is null
            """
        ).fetchall():
            _update_repo_id(conn, "reviews", ("review_id",), (row["review_id"],), str(row["repo_id"] or "").strip())
        for row in conn.execute(
            """
            select submission_id, changes.repo_id as repo_id
            from land_requests
            join changes using(change_id)
            where land_requests.repo_id is null
            """
        ).fetchall():
            _update_repo_id(conn, "land_requests", ("submission_id",), (row["submission_id"],), str(row["repo_id"] or "").strip())
        for row in conn.execute(
            """
            select attestations.patchset_id, patchsets.repo_id as repo_id
            from attestations
            join patchsets using(patchset_id)
            where attestations.repo_id is null
            """
        ).fetchall():
            _update_repo_id(conn, "attestations", ("patchset_id",), (row["patchset_id"],), str(row["repo_id"] or "").strip())
        for row in conn.execute(
            """
            select policy_decision_id, patchsets.repo_id as repo_id
            from policy_decisions
            join patchsets using(patchset_id)
            where policy_decisions.repo_id is null
            """
        ).fetchall():
            _update_repo_id(conn, "policy_decisions", ("policy_decision_id",), (row["policy_decision_id"],), str(row["repo_id"] or "").strip())
        for row in conn.execute(
            """
            select waiver_id, patchsets.repo_id as repo_id
            from waivers
            join patchsets using(patchset_id)
            where waivers.repo_id is null
            """
        ).fetchall():
            _update_repo_id(conn, "waivers", ("waiver_id",), (row["waiver_id"],), str(row["repo_id"] or "").strip())
        for row in conn.execute(
            """
            select planning_session_events.planning_session_id, planning_session_events.sequence, planning_sessions.repo_id as repo_id
            from planning_session_events
            join planning_sessions using(planning_session_id)
            where planning_session_events.repo_id is null
            """
        ).fetchall():
            _update_repo_id(
                conn,
                "planning_session_events",
                ("planning_session_id", "sequence"),
                (row["planning_session_id"], row["sequence"]),
                str(row["repo_id"] or "").strip(),
            )
        for row in conn.execute(
            """
            select session_events.session_id, session_events.sequence, sessions.repo_id as repo_id
            from session_events
            join sessions using(session_id)
            where session_events.repo_id is null
            """
        ).fetchall():
            _update_repo_id(
                conn,
                "session_events",
                ("session_id", "sequence"),
                (row["session_id"], row["sequence"]),
                str(row["repo_id"] or "").strip(),
            )
        for row in conn.execute(
            """
            select checkpoint_id, sessions.repo_id as repo_id
            from session_checkpoints
            join sessions using(session_id)
            where session_checkpoints.repo_id is null
            """
        ).fetchall():
            _update_repo_id(conn, "session_checkpoints", ("checkpoint_id",), (row["checkpoint_id"],), str(row["repo_id"] or "").strip())
        for row in conn.execute(
            """
            select stack_id, change_id, stacks.repo_id as repo_id
            from stack_changes
            join stacks using(stack_id)
            where stack_changes.repo_id is null
            """
        ).fetchall():
            _update_repo_id(
                conn,
                "stack_changes",
                ("stack_id", "change_id"),
                (row["stack_id"], row["change_id"]),
                str(row["repo_id"] or "").strip(),
            )
        conn.commit()


def _local_id_after_first_dash(value: str | None) -> str | None:
    text = str(value or "").strip()
    if not text:
        return None
    if "-" not in text:
        return text
    suffix = text.split("-", 1)[1].strip()
    return suffix or text


def _sequence_after_first_dash(value: str | None) -> int | None:
    suffix = _local_id_after_first_dash(value)
    if suffix is None:
        return None
    try:
        return int(suffix)
    except ValueError:
        return None


def _sequence_after_last_dash(value: str | None) -> int | None:
    text = str(value or "").strip()
    if "-" not in text:
        return None
    try:
        return int(text.rsplit("-", 1)[1])
    except ValueError:
        return None


def _update_local_key(conn, table_name: str, key_fields: tuple[str, ...], key_values: tuple[Any, ...], column_name: str, value: Any) -> None:
    if value is None or value == "":
        return
    where = " and ".join(f"{field} = ?" for field in key_fields)
    conn.execute(
        f"update {table_name} set {column_name} = ? where {column_name} is null and {where}",
        (value, *key_values),
    )


def _next_repo_sequence(conn, table_name: str, repo_id: str, column_name: str) -> int:
    row = conn.execute(
        f"select coalesce(max({column_name}), 0) as max_value from {table_name} where repo_id = ?",
        (repo_id,),
    ).fetchone()
    return int(row["max_value"] or 0) + 1


def _backfill_control_plane_local_keys(ctx: ServerContext) -> None:
    with connect(ctx) as conn:
        for row in conn.execute("select task_id from tasks where task_seq is null").fetchall():
            _update_local_key(conn, "tasks", ("task_id",), (row["task_id"],), "task_seq", _sequence_after_first_dash(row["task_id"]))
        for row in conn.execute("select change_id from changes where change_seq is null").fetchall():
            _update_local_key(conn, "changes", ("change_id",), (row["change_id"],), "change_seq", _sequence_after_first_dash(row["change_id"]))
        for row in conn.execute("select session_id from sessions where session_local_id is null").fetchall():
            _update_local_key(conn, "sessions", ("session_id",), (row["session_id"],), "session_local_id", _local_id_after_first_dash(row["session_id"]))
        for row in conn.execute("select planning_session_id from planning_sessions where planning_session_local_id is null").fetchall():
            _update_local_key(
                conn,
                "planning_sessions",
                ("planning_session_id",),
                (row["planning_session_id"],),
                "planning_session_local_id",
                _local_id_after_first_dash(row["planning_session_id"]),
            )
        for row in conn.execute("select checkpoint_id from session_checkpoints where checkpoint_local_id is null").fetchall():
            _update_local_key(
                conn,
                "session_checkpoints",
                ("checkpoint_id",),
                (row["checkpoint_id"],),
                "checkpoint_local_id",
                _local_id_after_first_dash(row["checkpoint_id"]),
            )
        next_land_seq_by_repo: dict[str, int] = {}
        for row in conn.execute(
            """
            select submission_id, repo_id
            from land_requests
            where land_seq is null
            order by created_at asc, submission_id asc
            """
        ).fetchall():
            repo_id = str(row["repo_id"] or "").strip()
            if not repo_id:
                continue
            next_land_seq = next_land_seq_by_repo.get(repo_id)
            if next_land_seq is None:
                next_land_seq = _next_repo_sequence(conn, "land_requests", repo_id, "land_seq")
            _update_local_key(
                conn,
                "land_requests",
                ("submission_id",),
                (row["submission_id"],),
                "land_seq",
                next_land_seq,
            )
            next_land_seq_by_repo[repo_id] = next_land_seq + 1
        for row in conn.execute("select stack_id from stacks where stack_seq is null").fetchall():
            _update_local_key(conn, "stacks", ("stack_id",), (row["stack_id"],), "stack_seq", _sequence_after_first_dash(row["stack_id"]))
        conn.commit()


def _active_waiver_rules(conn, patchset_id: str) -> set[str]:
    now = utc_now()
    rows = conn.execute(
        "select rule_name, expires_at from waivers where patchset_id = ? order by created_at desc",
        (patchset_id,),
    ).fetchall()
    out: set[str] = set()
    for row in rows:
        expires_at = row["expires_at"]
        if expires_at and expires_at < now:
            continue
        out.add(row["rule_name"])
    return out


def _policy_input_fingerprint(
    conn,
    patchset: dict[str, Any] | Any,
    change: dict[str, Any] | Any,
    *,
    lane: str,
    repo_policy: dict[str, Any] | None,
    attestation_row: Any | None,
    active_waiver_rules: set[str],
) -> str:
    review_stamp = conn.execute(
        "select coalesce(max(review_id), 0) as max_review_id from reviews where change_id = ? and patchset_id = ?",
        (change["change_id"], patchset["patchset_id"]),
    ).fetchone()
    payload = {
        "lane": lane,
        "risk_tier": str(change["risk_tier"] or "").strip(),
        "revision_snapshot_id": str(patchset["revision_snapshot_id"] or "").strip(),
        "patchset_author_mode": str(patchset["author_mode"] or "").strip(),
        "diff_stats_json": str(patchset["diff_stats_json"] or "").strip(),
        "repo_policy": repo_policy or {},
        "attestation_author_mode": str(attestation_row["author_mode"] or "").strip() if attestation_row is not None else "",
        "attestation_evaluation_summary_json": str(attestation_row["evaluation_summary_json"] or "").strip() if attestation_row is not None else "",
        "attestation_provenance_summary_json": str(attestation_row["provenance_summary_json"] or "").strip()
        if attestation_row is not None
        else "",
        "attestation_detail_json": str(attestation_row["detail_json"] or "").strip() if attestation_row is not None else "",
        "max_review_id": int(review_stamp["max_review_id"] or 0) if review_stamp is not None else 0,
        "active_waiver_rules": sorted(active_waiver_rules),
    }
    encoded = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def _current_patchset_row(conn, change_id: str):
    return conn.execute(
        "select * from patchsets where change_id = ? order by patchset_number desc limit 1",
        (change_id,),
    ).fetchone()


def _selected_patchset_row(conn, change_id: str):
    change = conn.execute("select selected_patchset_number from changes where change_id = ?", (change_id,)).fetchone()
    if change is None or change["selected_patchset_number"] is None:
        return None
    return conn.execute(
        "select * from patchsets where change_id = ? and patchset_number = ?",
        (change_id, change["selected_patchset_number"]),
    ).fetchone()


def _stack_ids_for_change(conn, change_id: str) -> list[str]:
    return [row["stack_id"] for row in conn.execute("select stack_id from stack_changes where change_id = ? order by stack_id", (change_id,)).fetchall()]


def _change_payload_from_row(ctx: ServerContext, conn, row: Any) -> dict[str, Any]:
    change_id = str(row["change_id"])
    out = dict(row)
    current = _current_patchset_row(conn, change_id)
    selected = _selected_patchset_row(conn, change_id)
    out["current_patchset_id"] = current["patchset_id"] if current is not None else None
    out["selected_patchset_id"] = selected["patchset_id"] if selected is not None else None
    out["stack_ids"] = _stack_ids_for_change(conn, change_id)
    return out


def _refresh_stack_state(conn, stack_id: str) -> str:
    stack = conn.execute("select * from stacks where stack_id = ?", (stack_id,)).fetchone()
    if stack is None:
        raise KeyError(f"Unknown stack: {stack_id}")
    if stack["status"] == "archived":
        return "archived"
    members = conn.execute(
        """
        select c.change_id, c.status, c.current_patchset_number
        from stack_changes sc
        join changes c on c.change_id = sc.change_id
        where sc.stack_id = ?
        order by sc.position asc
        """,
        (stack_id,),
    ).fetchall()
    if not members:
        status = "active"
    else:
        member_states = {row["status"] for row in members}
        if member_states == {"landed"}:
            status = "landed"
        elif "blocked" in member_states:
            status = "blocked"
        elif member_states.issubset({"landable", "landed"}):
            status = "ready_to_land"
        else:
            status = "active"
    conn.execute("update stacks set status = ?, updated_at = ? where stack_id = ?", (status, utc_now(), stack_id))
    return status


def _refresh_stacks_for_change(conn, change_id: str) -> None:
    for stack_id in _stack_ids_for_change(conn, change_id):
        _refresh_stack_state(conn, stack_id)


def _resolve_patchset_for_change(conn, change_id: str, patchset_id: str | None = None):
    if patchset_id:
        row = conn.execute("select * from patchsets where patchset_id = ? and change_id = ?", (patchset_id, change_id)).fetchone()
        if row is None:
            raise KeyError(f"Patchset {patchset_id} does not belong to change {change_id}")
        return row
    selected = _selected_patchset_row(conn, change_id)
    if selected is not None:
        return selected
    current = _current_patchset_row(conn, change_id)
    if current is None:
        raise KeyError(f"Change {change_id} has no published patchset")
    return current


def _ensure_change_mutable(change, action: str) -> None:
    status = change["status"]
    change_id = change["change_id"]
    if status == "archived":
        raise ValueError(f"Change {change_id} is archived and cannot {action}")
    if status == "landed":
        raise ValueError(f"Change {change_id} is landed and cannot {action}")


def _refresh_change_state(ctx: ServerContext, conn, change_id: str) -> str:
    change = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
    if change is None:
        raise KeyError(f"Unknown change: {change_id}")
    if change["status"] in {"landed", "archived"}:
        return change["status"]
    if change["current_patchset_number"] == 0:
        new_state = "draft"
    else:
        patchset = _current_patchset_row(conn, change_id)
        assert patchset is not None
        policy = _effective_policy_status(
            conn,
            patchset,
            lane=change["lane"] or lane_from_risk(change["risk_tier"]),
        )
        review = _review_summary(conn, change_id, patchset["patchset_id"])
        required = _required_approvals(policy["lane"])
        repo_name = _repo_name_for_repo_id(ctx, change.get("repo_id"), change["repo_name"])
        base_line_head = read_content_ref(ctx, repo_name, change["base_line"])
        stale = bool(base_line_head and base_line_head != patchset["base_snapshot_id"])
        if review["blocking_count"] > 0:
            new_state = "blocked"
        elif policy["decision"] == "hard_fail":
            new_state = "blocked"
        elif stale:
            new_state = "blocked"
        elif policy["decision"] == "pass" and review["approval_count"] >= required and not stale:
            new_state = "landable"
        elif review["approval_count"] >= required and policy["decision"] == "pass":
            new_state = "approved"
        elif policy["decision"] in {"pending", "soft_fail"}:
            new_state = "gated"
        else:
            new_state = "review"
    conn.execute("update changes set status = ?, updated_at = ? where change_id = ?", (new_state, utc_now(), change_id))
    _refresh_stacks_for_change(conn, change_id)
    return new_state


























def create_task(
    ctx: ServerContext,
    repo_name: str,
    title: str,
    intent: str,
    risk_tier: str,
    task_id: str | None = None,
    *,
    plan_id: str | None = None,
    origin_plan_revision_id: str | None = None,
    plan_item_ref: str | None = None,
    tracking_session: dict[str, Any] | None = None,
    actor_identity: str = "system",
    actor_type: str = "system_worker",
) -> dict:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    repo_namespace_prefix = _repo_id_namespace_prefix(ctx, repo_name)
    repo_id = _repo_id(ctx, repo_name)
    with connect(ctx) as conn:
        resolved_plan_id, resolved_origin_revision_id, resolved_plan_item_ref = _resolve_task_plan_linkage(
            ctx,
            conn,
            repo_name,
            plan_id=plan_id,
            origin_plan_revision_id=origin_plan_revision_id,
            plan_item_ref=plan_item_ref,
        )
        planning_state = "planned" if resolved_plan_id is not None else "unplanned"
        assert planning_state in TASK_PLANNING_STATES
        next_task_seq = _next_repo_sequence(conn, "tasks", repo_id, "task_seq")
        if task_id is None:
            task_seq = next_task_seq
            task_id = generate_namespaced_sequence_id(
                "T",
                task_seq,
                workflow_origin_namespace_prefix(REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX, repo_namespace_prefix),
            )
        else:
            requested_task_seq = _sequence_after_first_dash(task_id)
            task_seq = requested_task_seq if requested_task_seq is not None and requested_task_seq >= next_task_seq else next_task_seq
        existing = conn.execute("select * from tasks where task_id = ?", (task_id,)).fetchone()
        if existing is not None:
            row = dict(existing)
            if (
                row["repo_name"] == repo_name
                and row["title"] == title
                and row["intent"] == intent
                and row["risk_tier"] == risk_tier
                and row.get("planning_state") == planning_state
                and row.get("plan_id") == resolved_plan_id
                and row.get("origin_plan_revision_id") == resolved_origin_revision_id
                and row.get("plan_item_ref") == resolved_plan_item_ref
            ):
                session = _ensure_task_tracking_session_conn(
                    ctx,
                    conn,
                    row,
                    tracking_session=tracking_session,
                    actor_identity=actor_identity,
                    actor_type=actor_type,
                    provisioned_by="task.create",
                )
                conn.commit()
                return _attach_task_tracking_payload(row, session, provisioned_by="task.create")
            raise ValueError(f"Task {task_id} already exists with different fields")
        if resolved_plan_id is not None and resolved_plan_item_ref is not None:
            linked_task = conn.execute(
                """
                select task_id, origin_plan_revision_id, status
                from tasks
                where """
                + _repo_scope_predicate()
                + """ and plan_id = ? and plan_item_ref = ?
                order by task_seq desc, created_at desc
                limit 1
                """,
                (repo_id, repo_name, resolved_plan_id, resolved_plan_item_ref),
            ).fetchone()
            if linked_task is not None:
                linked_task_row = dict(linked_task)
                linked_task_id = str(linked_task_row.get("task_id") or "").strip()
                linked_status = str(linked_task_row.get("status") or "unknown").strip() or "unknown"
                linked_revision_id = str(linked_task_row.get("origin_plan_revision_id") or "").strip()
                revision_note = f" from revision {linked_revision_id}" if linked_revision_id else ""
                raise ValueError(
                    f"Plan item ref {resolved_plan_item_ref!r} on plan {resolved_plan_id}{revision_note} "
                    f"is already linked to task {linked_task_id} (status: {linked_status}). "
                    "Open a new plan item ref or a new execution-plan revision instead of binding a new task to an older dispatched ref."
                )
        now = utc_now()
        plan_linked_at = now if resolved_plan_id is not None else None
        conn.execute(
            """
            insert into tasks(
                task_id, repo_name, repo_id, task_seq, title, intent, risk_tier, planning_state, plan_id, origin_plan_revision_id, plan_item_ref, plan_linked_at, status, created_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', ?)
            """,
            (
                task_id,
                repo_name,
                repo_id,
                task_seq,
                title,
                intent,
                risk_tier,
                planning_state,
                resolved_plan_id,
                resolved_origin_revision_id,
                resolved_plan_item_ref,
                plan_linked_at,
                now,
            ),
        )
        record_event(
            conn,
            "task.created",
            "task",
            task_id,
            {
                "repo_name": repo_name,
                "title": title,
                "planning_state": planning_state,
                "plan_id": resolved_plan_id,
                "origin_plan_revision_id": resolved_origin_revision_id,
                "plan_item_ref": resolved_plan_item_ref,
                "plan_linked_at": plan_linked_at,
            },
            actor_identity=actor_identity,
            actor_type=actor_type,
        )
        row = conn.execute("select * from tasks where task_id = ?", (task_id,)).fetchone()
        assert row is not None
        task = dict(row)
        session = _ensure_task_tracking_session_conn(
            ctx,
            conn,
            task,
            tracking_session=tracking_session,
            actor_identity=actor_identity,
            actor_type=actor_type,
            provisioned_by="task.create",
        )
        conn.commit()
        return _attach_task_tracking_payload(task, session, provisioned_by="task.create")


def list_tasks(ctx: ServerContext, repo_name: str) -> list[dict]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    repo_id = _repo_id(ctx, repo_name)
    with connect(ctx) as conn:
        rows = [
            dict(r)
            for r in conn.execute(
                "select * from tasks where " + _repo_scope_predicate() + " order by created_at desc",
                (repo_id, repo_name),
            )
        ]
    return rows


def list_plan_linked_tasks(ctx: ServerContext, repo_name: str) -> list[dict]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    repo_id = _repo_id(ctx, repo_name)
    with connect(ctx) as conn:
        rows = [
            dict(r)
            for r in conn.execute(
                """
                select task_id, plan_id, origin_plan_revision_id, plan_item_ref, status, created_at
                from tasks
                where """
                + _repo_scope_predicate()
                + """ and plan_id is not null
                order by created_at desc
                """,
                (repo_id, repo_name),
            )
        ]
    return rows


def _repo_scoped_sequence_ref(value: str | None) -> int | None:
    text = str(value or "").strip()
    if not text:
        return None
    if text.isdigit():
        return int(text)
    return None


def _assert_repo_scope(ctx: ServerContext, repo_name: str, repo_id: str | None = None) -> str:
    resolved_repo_id = _repo_id(ctx, repo_name)
    expected_repo_id = str(repo_id or "").strip()
    if expected_repo_id and expected_repo_id != resolved_repo_id:
        raise ValueError(
            f"Repository scope mismatch for {repo_name}: repo_id {expected_repo_id} does not match {resolved_repo_id}"
        )
    return resolved_repo_id


def get_task_for_repo(ctx: ServerContext, repo_name: str, task_ref: str) -> dict:
    repo_id = _repo_id(ctx, repo_name)
    with connect(ctx) as conn:
        row = conn.execute(
            "select * from tasks where repo_id = ? and task_id = ?",
            (repo_id, task_ref),
        ).fetchone()
        if row is None:
            task_seq = _repo_scoped_sequence_ref(task_ref)
            if task_seq is not None:
                row = conn.execute(
                    "select * from tasks where repo_id = ? and task_seq = ?",
                    (repo_id, task_seq),
                ).fetchone()
    if row is None:
        raise KeyError(f"Unknown task {task_ref} for repository {repo_name}")
    return dict(row)


def get_task(ctx: ServerContext, task_id: str) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select * from tasks where task_id = ?", (task_id,)).fetchone()
    if row is None:
        raise KeyError(f"Unknown task: {task_id}")
    return dict(row)


def close_task(ctx: ServerContext, task_id: str, status: str) -> dict:
    normalized_status = normalize_task_status(status)
    if normalized_status not in TASK_REMOTE_CLOSE_TARGET_STATUSES:
        raise ValueError(f"Unsupported task close status: {status}")
    with connect(ctx) as conn:
        row = conn.execute("select * from tasks where task_id = ?", (task_id,)).fetchone()
        if row is None:
            raise KeyError(f"Unknown task: {task_id}")
        task = dict(row)
        task_status = normalize_task_status(task.get("status"))
        assert task_status is not None
        if task_status == normalized_status:
            return task
        if task_status != "active":
            raise ValueError(f"Task {task_id} is already {task_status}; reopening is not supported")

        change_rows = [
            dict(r)
            for r in conn.execute(
                "select change_id, status from changes where task_id = ? order by created_at asc",
                (task_id,),
            )
        ]
        if normalized_status == TASK_STATUS_COMPLETED:
            open_changes = [f"{row['change_id']} ({row['status']})" for row in change_rows if row["status"] not in {"landed", "archived"}]
            if open_changes:
                raise ValueError(f"Task {task_id} cannot be completed while changes are still open: {', '.join(open_changes)}")
        if is_task_abandoned_status(normalized_status):
            landed_changes = [row["change_id"] for row in change_rows if row["status"] == "landed"]
            if landed_changes:
                raise ValueError(f"Task {task_id} cannot be abandoned because landed changes exist: {', '.join(landed_changes)}")

        conn.execute("update tasks set status = ? where task_id = ?", (normalized_status, task_id))
        record_event(
            conn,
            "task.closed",
            "task",
            task_id,
            {"repo_name": task["repo_name"], "status": normalized_status, "previous_status": task["status"]},
        )
        refreshed_row = conn.execute("select * from tasks where task_id = ?", (task_id,)).fetchone()
        assert refreshed_row is not None
        refreshed_task = dict(refreshed_row)
        _ensure_task_tracking_session_conn(
            ctx,
            conn,
            refreshed_task,
            tracking_session=None,
            actor_identity="system",
            actor_type="system_worker",
            provisioned_by="task.close",
        )
        conn.commit()
        row = conn.execute("select * from tasks where task_id = ?", (task_id,)).fetchone()
    assert row is not None
    return dict(row)


def _restart_change_conn(conn, change_row: dict[str, Any]) -> dict[str, Any]:
    change_status = str(change_row.get("status") or "").strip()
    if change_status == "draft":
        return change_row
    if change_status != "archived":
        raise ValueError(
            f"Change {change_row['change_id']} is `{change_status or 'unknown'}`; restart only supports archived changes."
        )
    now = utc_now()
    conn.execute(
        "update changes set status = 'draft', updated_at = ? where change_id = ?",
        (now, str(change_row["change_id"])),
    )
    refreshed = conn.execute("select * from changes where change_id = ?", (str(change_row["change_id"]),)).fetchone()
    assert refreshed is not None
    return dict(refreshed)


def restart_task(ctx: ServerContext, task_id: str) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select * from tasks where task_id = ?", (task_id,)).fetchone()
        if row is None:
            raise KeyError(f"Unknown task: {task_id}")
        task = dict(row)
        task_status = normalize_task_status(task.get("status"))
        assert task_status is not None
        if not is_task_abandoned_status(task_status):
            raise ValueError(f"Task {task_id} is `{task_status}`; restart only supports task canceled lineage.")

        change_rows = [
            dict(r)
            for r in conn.execute(
                "select * from changes where task_id = ? order by created_at asc",
                (task_id,),
            )
        ]
        landed_change = next((change_row for change_row in change_rows if str(change_row.get("status") or "").strip() == "landed"), None)
        if landed_change is not None:
            raise ValueError(
                f"Task {task_id} cannot be restarted because landed change {landed_change['change_id']} already exists."
            )
        archived_changes = [change_row for change_row in change_rows if str(change_row.get("status") or "").strip() == "archived"]
        open_changes = [
            change_row
            for change_row in change_rows
            if str(change_row.get("status") or "").strip() not in {"archived", "superseded"}
        ]
        if not open_changes and len(archived_changes) > 1:
            archived_ids = ", ".join(str(change_row["change_id"]) for change_row in archived_changes)
            raise ValueError(
                f"Task {task_id} has multiple archived changes ({archived_ids}); restart only supports one archived change."
            )

        now = utc_now()
        conn.execute("update tasks set status = 'active' where task_id = ?", (task_id,))
        record_event(
            conn,
            "task.restarted",
            "task",
            task_id,
            {"repo_name": task["repo_name"], "status": "active", "previous_status": task["status"]},
        )
        restarted_change: dict[str, Any] | None = None
        if not open_changes and len(archived_changes) == 1:
            restarted_change = _restart_change_conn(conn, archived_changes[0])
            record_event(
                conn,
                "change.restarted",
                "change",
                str(restarted_change["change_id"]),
                {
                    "repo_name": task["repo_name"],
                    "status": "draft",
                    "previous_status": archived_changes[0]["status"],
                    "task_id": task_id,
                },
            )

        refreshed_row = conn.execute("select * from tasks where task_id = ?", (task_id,)).fetchone()
        assert refreshed_row is not None
        refreshed_task = dict(refreshed_row)
        _ensure_task_tracking_session_conn(
            ctx,
            conn,
            refreshed_task,
            tracking_session=None,
            actor_identity="system",
            actor_type="system_worker",
            provisioned_by="task.restart",
        )
        conn.commit()
        payload = dict(refreshed_task)
        if restarted_change is not None:
            payload["change"] = _change_payload_from_row(ctx, conn, restarted_change)
        return payload


def create_change(
    ctx: ServerContext,
    repo_name: str,
    task_id: str,
    title: str,
    base_line: str,
    risk_tier: str,
    change_id: str | None = None,
    *,
    fork_snapshot_id: str | None = None,
    forked_from_line: str | None = None,
) -> dict:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    base_line_row = get_content_line(ctx, repo_name, base_line)
    repo_namespace_prefix = _repo_id_namespace_prefix(ctx, repo_name)
    repo_id = _repo_id(ctx, repo_name)
    with connect(ctx) as conn:
        task = conn.execute("select task_id, status from tasks where task_id = ? and " + _repo_scope_predicate(), (task_id, repo_id, repo_name)).fetchone()
        if task is None:
            raise KeyError(f"Unknown task {task_id} for repository {repo_name}")
        if task["status"] != "active":
            raise ValueError(f"Task {task_id} is {task['status']} and cannot accept new changes")
        next_change_seq = _next_repo_sequence(conn, "changes", repo_id, "change_seq")
        if change_id is None:
            change_seq = next_change_seq
            change_id = generate_namespaced_sequence_id(
                "C",
                change_seq,
                workflow_origin_namespace_prefix(REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX, repo_namespace_prefix),
            )
        else:
            requested_change_seq = _sequence_after_first_dash(change_id)
            change_seq = (
                requested_change_seq
                if requested_change_seq is not None and requested_change_seq >= next_change_seq
                else next_change_seq
            )
        now = utc_now()
        lane = lane_from_risk(risk_tier)
        resolved_fork_snapshot_id = _normalize_optional_text(fork_snapshot_id) or _normalize_optional_text(base_line_row.get("head_snapshot_id"))
        resolved_forked_from_line = _normalize_optional_text(forked_from_line) or base_line
        existing = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
        if existing is not None:
            row = dict(existing)
            if (
                row["repo_name"] == repo_name
                and row["task_id"] == task_id
                and row["title"] == title
                and row["base_line"] == base_line
                and _normalize_optional_text(row.get("fork_snapshot_id")) == resolved_fork_snapshot_id
                and _normalize_optional_text(row.get("forked_from_line")) == resolved_forked_from_line
                and row["risk_tier"] == risk_tier
                and row["lane"] == lane
            ):
                return row
            raise ValueError(f"Change {change_id} already exists with different fields")
        conn.execute(
            "insert into changes(change_id, repo_name, repo_id, change_seq, task_id, title, base_line, fork_snapshot_id, forked_from_line, risk_tier, lane, status, current_patchset_number, created_at, updated_at) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'draft', 0, ?, ?)",
            (
                change_id,
                repo_name,
                repo_id,
                change_seq,
                task_id,
                title,
                base_line,
                resolved_fork_snapshot_id,
                resolved_forked_from_line,
                risk_tier,
                lane,
                now,
                now,
            ),
        )
        record_event(
            conn,
            "change.created",
            "change",
            change_id,
            {
                "repo_name": repo_name,
                "task_id": task_id,
                "title": title,
                "base_line": base_line,
                "fork_snapshot_id": resolved_fork_snapshot_id,
                "forked_from_line": resolved_forked_from_line,
                "lane": lane,
            },
        )
        conn.commit()
        row = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
    return dict(row)


def list_changes(ctx: ServerContext, repo_name: str) -> list[dict]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    repo_id = _repo_id(ctx, repo_name)
    with connect(ctx) as conn:
        rows = [
            dict(r)
            for r in conn.execute(
                "select * from changes where " + _repo_scope_predicate() + " order by updated_at desc",
                (repo_id, repo_name),
            )
        ]
    return rows


def get_change_for_repo(ctx: ServerContext, repo_name: str, change_ref: str) -> dict:
    repo_id = _repo_id(ctx, repo_name)
    with connect(ctx) as conn:
        row = conn.execute(
            "select * from changes where repo_id = ? and change_id = ?",
            (repo_id, change_ref),
        ).fetchone()
        if row is None:
            change_seq = _repo_scoped_sequence_ref(change_ref)
            if change_seq is not None:
                row = conn.execute(
                    "select * from changes where repo_id = ? and change_seq = ?",
                    (repo_id, change_seq),
                ).fetchone()
        if row is None:
            raise KeyError(f"Unknown change {change_ref} for repository {repo_name}")
        return _change_payload_from_row(ctx, conn, row)


def get_change(ctx: ServerContext, change_id: str) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
        if row is None:
            raise KeyError(f"Unknown change: {change_id}")
        return _change_payload_from_row(ctx, conn, row)


def close_change(ctx: ServerContext, change_id: str, status: str) -> dict:
    if status != "archived":
        raise ValueError(f"Unsupported change close status: {status}")
    with connect(ctx) as conn:
        row = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
        if row is None:
            raise KeyError(f"Unknown change: {change_id}")
        change = dict(row)
        if change["status"] == status:
            return get_change(ctx, change_id)
        if change["status"] == "landed":
            raise ValueError(f"Change {change_id} is landed and cannot be archived")
        conn.execute("update changes set status = 'archived', updated_at = ? where change_id = ?", (utc_now(), change_id))
        record_event(
            conn,
            "change.closed",
            "change",
            change_id,
            {"repo_name": change["repo_name"], "status": "archived", "previous_status": change["status"]},
        )
        _refresh_stacks_for_change(conn, change_id)
        conn.commit()
    return get_change(ctx, change_id)


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


def evaluate_policy(ctx: ServerContext, patchset_id: str) -> dict:
    with connect(ctx) as conn:
        patchset = conn.execute("select * from patchsets where patchset_id = ?", (patchset_id,)).fetchone()
        if patchset is None:
            raise KeyError(f"Unknown patchset: {patchset_id}")
        change = conn.execute("select * from changes where change_id = ?", (patchset["change_id"],)).fetchone()
        assert change is not None
        lane = change["lane"] or lane_from_risk(change["risk_tier"])
        repo_name = _repo_name_for_repo_id(ctx, change.get("repo_id"), change["repo_name"])
        repository = get_content_repository(ctx, repo_name)
        repo_policy = repository.get("policy", {})
        attestation_row = conn.execute("select * from attestations where patchset_id = ?", (patchset_id,)).fetchone()
        waivers = _active_waiver_rules(conn, patchset_id)
        policy_context = _policy_context_for_patchset(repo_policy, patchset, attestation_row)
        effective_requirements = dict(policy_context["effective_requirements"] or {})
        requires_code_review_summary = _requires_code_review_summary(policy_context)
        effective_requirements["require_code_review_summary"] = requires_code_review_summary
        provenance_summary = policy_context["provenance_summary"]
        input_fingerprint = _policy_input_fingerprint(
            conn,
            patchset,
            change,
            lane=lane,
            repo_policy=repo_policy,
            attestation_row=attestation_row,
            active_waiver_rules=waivers,
        )
        latest_status = latest_policy_status(conn, patchset_id)
        evaluation_state = str(patchset["evaluation_state"] or "pending").strip() or "pending"
        if (
            evaluation_state != "pending"
            and latest_status is not None
            and str(latest_status.get("decision") or "").strip() == evaluation_state
            and str(latest_status.get("input_fingerprint") or "").strip() == input_fingerprint
        ):
            return {
                "patchset_id": patchset_id,
                "lane": str(latest_status.get("lane") or lane).strip() or lane,
                "decision": evaluation_state,
                "checks": list(latest_status.get("checks") or []),
                "evaluated_at": latest_status.get("evaluated_at"),
                "policy_id": repo_policy.get("policy_id", "prototype"),
                "content_class": policy_context["content_class"],
                "author_class": policy_context["author_class"],
                "effective_requirements": effective_requirements,
                "matched_overrides": policy_context["matched_overrides"],
            }

        review = _review_summary(conn, change["change_id"], patchset_id)

        checks: list[dict[str, Any]] = []
        decision = "pass"
        waived_any = False

        def allows_waiver(rule_name: str) -> bool:
            normalized_rule = str(rule_name or "").strip()
            if normalized_rule in {"tests", "require_tests"}:
                return False
            if normalized_rule.startswith("ci_patchset_suite_"):
                return False
            return True

        def add_check(rule_name: str, status: str, message: str | None = None, *, label: str | None = None) -> None:
            nonlocal decision, waived_any
            final_status = status
            if status == "hard_fail" and rule_name in waivers and allows_waiver(rule_name):
                final_status = "waived"
                waived_any = True
            effective_label = label or RULE_LABELS.get(rule_name, rule_name)
            checks.append({"name": rule_name, "label": effective_label, "status": final_status, "message": message or effective_label})
            if final_status == "hard_fail":
                decision = "hard_fail"
            elif final_status == "pending" and decision != "hard_fail":
                decision = "pending"
            elif final_status == "soft_fail" and decision not in {"hard_fail", "pending"}:
                decision = "soft_fail"

        if bool(effective_requirements.get("require_attestation", True)):
            if attestation_row is None:
                add_check("require_attestation", "pending", "Attestation is required before landing")
            else:
                add_check("require_attestation", "pass")
        else:
            add_check("require_attestation", "not_required", "Attestation is optional by repository policy")

        if bool(effective_requirements.get("require_ai_provenance", False)):
            if attestation_row is None:
                add_check("ai_provenance", "pending", "AI provenance is required before landing")
            elif provenance_summary.get("policy_readable"):
                add_check("ai_provenance", "pass")
            else:
                missing_fields = list(provenance_summary.get("missing_fields") or [])
                detail = ", ".join(missing_fields) if missing_fields else "minimum provenance fields are missing"
                add_check("ai_provenance", "pending", f"AI provenance is incomplete: {detail}")
        else:
            add_check("ai_provenance", "not_required", "AI provenance is optional by repository policy")

        if requires_code_review_summary:
            if int(review.get("code_review_summary_count") or 0) > 0:
                add_check("code_review_summary", "pass")
            else:
                add_check(
                    "code_review_summary",
                    "pending",
                    "Agent-prepared code review summary is required before landing",
                )
        else:
            add_check(
                "code_review_summary",
                "not_required",
                "Code review summary is not required for this patchset by repository policy",
            )

        evaluation = json.loads(attestation_row["evaluation_summary_json"]) if attestation_row is not None else {}
        for key in ("tests", "lint", "security_scan", "license_scan"):
            required = bool(effective_requirements.get(POLICY_REQUIREMENT_MAP[key], False))
            value = evaluation.get(key)
            if required:
                value = evaluation.get(key)
                if value == "pass":
                    add_check(key, "pass")
                elif value in {"fail", "failed"}:
                    add_check(key, "hard_fail", f"{RULE_LABELS.get(key, key)}")
                else:
                    add_check(key, "pending", f"{RULE_LABELS.get(key, key)}")
            else:
                if value == "pass":
                    add_check(key, "pass", f"{RULE_LABELS.get(key, key)} (optional)")
                elif value in {"fail", "failed"}:
                    add_check(key, "optional_fail", f"{RULE_LABELS.get(key, key)} (optional)")
                else:
                    add_check(key, "not_required", f"{RULE_LABELS.get(key, key)} not required by repository policy")

        ci_rollout = _ci_rollout_for_patchset(ctx, patchset, attestation_row)
        if ci_rollout is not None:
            add_check(
                "ci_rollout_phase",
                "pass",
                _ci_rollout_summary_message(ci_rollout),
                label="CI rollout phase",
            )
            for suite_check in _ci_rollout_patchset_suite_checks(ci_rollout):
                add_check(
                    suite_check["name"],
                    suite_check["status"],
                    suite_check["message"],
                    label=suite_check.get("label"),
                )

        required_approvals = _required_approvals(lane)
        if review["approval_count"] >= required_approvals:
            add_check("required_human_review", "pass")
        else:
            add_check("required_human_review", "pending", f"{required_approvals} approval(s) required for lane {lane}")

        if decision == "pass" and waived_any:
            decision = "waived"

        now = utc_now()
        repo_id = str(patchset["repo_id"] or "").strip() or _repo_id(ctx, change["repo_name"])
        conn.execute(
            "insert into policy_decisions(repo_id, patchset_id, lane, decision, checks_json, input_fingerprint, created_at) values (?, ?, ?, ?, ?, ?, ?)",
            (repo_id, patchset_id, lane, decision, json.dumps(checks, sort_keys=True), input_fingerprint, now),
        )
        conn.execute("update patchsets set evaluation_state = ? where patchset_id = ?", (decision, patchset_id))
        conn.execute("update changes set lane = ?, updated_at = ? where change_id = ?", (lane, now, change["change_id"]))
        record_event(conn, "policy.evaluated", "patchset", patchset_id, {"patchset_id": patchset_id, "lane": lane, "decision": decision})
        _refresh_change_state(ctx, conn, change["change_id"])
        conn.commit()
    return {
        "patchset_id": patchset_id,
        "lane": lane,
        "decision": decision,
        "checks": checks,
        "evaluated_at": now,
        "policy_id": repo_policy.get("policy_id", "prototype"),
        "content_class": policy_context["content_class"],
        "author_class": policy_context["author_class"],
        "effective_requirements": effective_requirements,
        "matched_overrides": policy_context["matched_overrides"],
    }


def get_policy_status(ctx: ServerContext, patchset_id: str) -> dict:
    with connect(ctx) as conn:
        patchset = conn.execute(
            """
            select p.*, c.repo_name, c.lane, c.risk_tier
            from patchsets p
            join changes c on c.change_id = p.change_id
            where p.patchset_id = ?
            """,
            (patchset_id,),
        ).fetchone()
        attestation_row = conn.execute("select * from attestations where patchset_id = ?", (patchset_id,)).fetchone()
        if patchset is None:
            raise KeyError(f"Unknown patchset: {patchset_id}")
        lane = patchset["lane"] or lane_from_risk(patchset["risk_tier"])
        status = _effective_policy_status(conn, patchset, lane=lane)
    repository = get_content_repository(ctx, _repo_name_for_repo_id(ctx, patchset["repo_id"], patchset["repo_name"]))
    repo_policy = repository.get("policy", {})
    policy_context = _policy_context_for_patchset(repo_policy, patchset, attestation_row)
    policy_context["effective_requirements"]["require_code_review_summary"] = _requires_code_review_summary(policy_context)
    status["policy_id"] = repo_policy.get("policy_id", "prototype")
    status["content_class"] = policy_context["content_class"]
    status["author_class"] = policy_context["author_class"]
    status["effective_requirements"] = policy_context["effective_requirements"]
    status["matched_overrides"] = policy_context["matched_overrides"]
    return status


def create_waiver(ctx: ServerContext, patchset_id: str, rule_name: str, reason: str, expires_at: str | None = None, *, inline: bool = True) -> dict:
    normalized_rule_name = str(rule_name or "").strip()
    if normalized_rule_name in {"tests", "require_tests"} or normalized_rule_name.startswith("ci_patchset_suite_"):
        raise ValueError(
            f"CI-backed rule `{normalized_rule_name}` cannot be waived. "
            "Fix the CI failure and rerun the required checks to `pass` before remote land."
        )
    with connect(ctx) as conn:
        patchset = conn.execute("select change_id, repo_id from patchsets where patchset_id = ?", (patchset_id,)).fetchone()
        if patchset is None:
            raise KeyError(f"Unknown patchset: {patchset_id}")
        count = conn.execute("select count(*) as c from waivers where patchset_id = ?", (patchset_id,)).fetchone()["c"]
        waiver_id = f"W-{patchset_id.split('-', 1)[1]}-{count + 1}"
        now = utc_now()
        conn.execute(
            "insert into waivers(waiver_id, repo_id, patchset_id, rule_name, reason, expires_at, created_at) values (?, ?, ?, ?, ?, ?, ?)",
            (waiver_id, str(patchset["repo_id"] or ""), patchset_id, normalized_rule_name, reason, expires_at, now),
        )
        record_event(conn, "policy.waived", "patchset", patchset_id, {"patchset_id": patchset_id, "rule_name": normalized_rule_name, "reason": reason})
        change_id = patchset["change_id"]
        conn.commit()
    base = {"waiver_id": waiver_id, "patchset_id": patchset_id, "rule_name": normalized_rule_name, "reason": reason, "expires_at": expires_at, "created_at": now, "change_id": change_id}
    if inline:
        base["policy"] = evaluate_policy(ctx, patchset_id)
    return base

def reconcile_repository(ctx: ServerContext, repo_name: str, *, repair: bool = False) -> dict:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    with connect(ctx) as conn:
        with connect_content(ctx) as content_conn:
            drifts: list[dict[str, Any]] = []
            repaired: list[dict[str, Any]] = []

            lines = list_content_lines(ctx, repo_name)
            for line in lines:
                head = line.get("head_snapshot_id")
                if head and not snapshot_exists(ctx, head):
                    item = {"type": "line_missing_snapshot", "line": line["line_name"], "snapshot_id": head}
                    drifts.append(item)
                    if repair:
                        update_content_line(ctx, repo_name, line["line_name"], None)
                        repaired.append({**item, "repaired": True, "new_head_snapshot_id": None})

            changes = conn.execute(
                "select * from changes where " + _repo_scope_predicate() + " order by change_id asc",
                (_repo_id(ctx, repo_name), repo_name),
            ).fetchall()
            for change in changes:
                latest = conn.execute(
                    "select patchset_number from patchsets where change_id = ? order by patchset_number desc limit 1",
                    (change["change_id"],),
                ).fetchone()
                latest_num = int(latest["patchset_number"]) if latest is not None else 0
                if int(change["current_patchset_number"]) != latest_num:
                    item = {
                        "type": "change_current_patchset_mismatch",
                        "change_id": change["change_id"],
                        "stored": int(change["current_patchset_number"]),
                        "expected": latest_num,
                    }
                    drifts.append(item)
                    if repair:
                        conn.execute(
                            "update changes set current_patchset_number = ?, updated_at = ? where change_id = ?",
                            (latest_num, utc_now(), change["change_id"]),
                        )
                        repaired.append({**item, "repaired": True})

                selected = change["selected_patchset_number"]
                if selected is not None:
                    row = conn.execute(
                        "select 1 from patchsets where change_id = ? and patchset_number = ?",
                        (change["change_id"], int(selected)),
                    ).fetchone()
                    if row is None:
                        item = {
                            "type": "change_selected_patchset_missing",
                            "change_id": change["change_id"],
                            "selected_patchset_number": int(selected),
                        }
                        drifts.append(item)
                        if repair:
                            conn.execute(
                                "update changes set selected_patchset_number = case when ? > 0 then ? else null end, updated_at = ? where change_id = ?",
                                (latest_num, latest_num, utc_now(), change["change_id"]),
                            )
                            repaired.append({**item, "repaired": True, "new_selected_patchset_number": latest_num or None})

            patchsets = conn.execute(
                """
                select p.patchset_id, p.base_snapshot_id, p.revision_snapshot_id
                from patchsets p
                join changes c on c.change_id = p.change_id
                where """
                + _repo_scope_predicate(alias="c")
                + """
                order by p.patchset_id asc
                """,
                (_repo_id(ctx, repo_name), repo_name),
            ).fetchall()
            for patchset in patchsets:
                for kind in ("base_snapshot_id", "revision_snapshot_id"):
                    snapshot_id = patchset[kind]
                    if snapshot_id and not snapshot_exists(ctx, snapshot_id):
                        drifts.append({
                            "type": "patchset_missing_snapshot",
                            "patchset_id": patchset["patchset_id"],
                            "field": kind,
                            "snapshot_id": snapshot_id,
                        })

            storage_signal_payload = repository_storage_signals(ctx, repo_name)
            for signal in storage_signal_payload["signals"]:
                drifts.append(signal)
                if not repair or not signal.get("repairable"):
                    continue
                if signal["type"] == "blob_storage_kind_mismatch":
                    content_conn.execute(
                        """
                        update blobs
                        set storage_kind = ?,
                            pack_entry_type = ?,
                            pack_base_blob_id = ?,
                            pack_chain_depth = ?,
                            pruned_at = ?
                        where blob_id = ?
                        """,
                        (
                            signal["expected_storage_kind"],
                            signal.get("expected_pack_entry_type"),
                            signal.get("expected_pack_base_blob_id"),
                            signal.get("expected_pack_chain_depth"),
                            utc_now(),
                            signal["blob_id"],
                        ),
                    )
                    repaired.append({**signal, "repaired": True})
                    continue
                if signal["type"] == "blob_pack_entry_type_mismatch":
                    content_conn.execute("update blobs set pack_entry_type = ? where blob_id = ?", (signal.get("expected"), signal["blob_id"]))
                    repaired.append({**signal, "repaired": True})
                    continue
                if signal["type"] == "blob_pack_base_mismatch":
                    content_conn.execute("update blobs set pack_base_blob_id = ? where blob_id = ?", (signal.get("expected"), signal["blob_id"]))
                    repaired.append({**signal, "repaired": True})
                    continue
                if signal["type"] == "blob_pack_chain_depth_mismatch":
                    content_conn.execute("update blobs set pack_chain_depth = ? where blob_id = ?", (signal.get("expected"), signal["blob_id"]))
                    repaired.append({**signal, "repaired": True})

            for change in changes:
                _refresh_change_state(ctx, conn, change["change_id"])
            stack_ids = [
                row["stack_id"]
                for row in conn.execute(
                    "select stack_id from stacks where " + _repo_scope_predicate(),
                    (_repo_id(ctx, repo_name), repo_name),
                ).fetchall()
            ]
            for stack_id in stack_ids:
                _refresh_stack_state(conn, stack_id)

            record_event(
                conn,
                "reconciliation.completed",
                "repository",
                repo_name,
                {
                    "repo_name": repo_name,
                    "repair": repair,
                    "drift_count": len(drifts),
                    "repaired_count": len(repaired),
                    "storage_drift_count": storage_signal_payload["summary"]["drift_count"],
                    "storage_repairable_drift_count": storage_signal_payload["summary"]["repairable_drift_count"],
                },
            )
            content_conn.commit()
            conn.commit()
    return {
        "repo_name": repo_name,
        "repair": repair,
        "drifts": drifts,
        "repaired": repaired,
        "drift_count": len(drifts),
        "repaired_count": len(repaired),
        "storage_signals_summary": storage_signal_payload["summary"],
    }
from .store.sessions import (
    _checkpoint_row,
    _session_event_row,
    _session_row,
    append_session_event,
    close_session,
    create_session,
    create_session_checkpoint,
    get_session,
    get_session_for_repo,
    get_session_checkpoint,
    get_session_checkpoint_for_repo,
    list_session_checkpoints,
    list_session_events,
    list_sessions,
    resume_session,
)
from .store.task_tracking import (
    _attach_task_tracking_payload,
    _ensure_task_tracking_session_conn,
    backfill_task_tracking_sessions,
    ensure_task_tracking_session,
)
from .store.stacks import (
    add_change_to_stack,
    create_stack,
    get_stack,
    get_stack_graph,
    list_stacks,
    remove_change_from_stack,
    reorder_stack_change,
    update_stack,
)
