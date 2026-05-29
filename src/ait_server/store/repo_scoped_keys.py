from __future__ import annotations

from typing import Any

from ..server_content import connect as connect_content
from ..server_control import connect
from ..server_paths import ServerContext
from .repo_ops import _repo_id


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
