from __future__ import annotations

import re

from ait_protocol.common import connect_sqlite, generate_namespaced_sequence_id, utc_now

from .repo_paths import RepoContext

LOCAL_IDENTITY_SOURCE_SEQUENCE = "local_sequence"
LOCAL_IDENTITY_SOURCE_LEGACY = "legacy_external"
_TASK_SEQUENCE_RE = re.compile(r"^[A-Z0-9]*T-(\d+)$")
_CHANGE_SEQUENCE_RE = re.compile(r"^[A-Z0-9]*C-(\d+)$")
_WORKFLOW_SEQUENCE_FLOOR_META_PREFIX = "workflow_sequence_floor"
_NON_BLOCKING_REMOTE_ALIAS_KINDS = frozenset({"published_remote_id"})


def _connect_control(ctx: RepoContext):
    return connect_sqlite(ctx.control_db_path)


def _ensure_column(conn, table_name: str, column_name: str, ddl: str) -> None:
    columns = {row[1] for row in conn.execute(f"pragma table_info({table_name})")}
    if column_name in columns:
        return
    conn.execute(f"alter table {table_name} add column {column_name} {ddl}")


def _workflow_sequence_from_id(value: str | None, *, family: str) -> int | None:
    text = str(value or "").strip().upper()
    if not text:
        return None
    matcher = _TASK_SEQUENCE_RE if family == "T" else _CHANGE_SEQUENCE_RE
    match = matcher.fullmatch(text)
    if match is None:
        return None
    return int(match.group(1))


def workflow_sequence_from_id(value: str | None, *, family: str) -> int | None:
    return _workflow_sequence_from_id(value, family=family)


def _workflow_sequence_floor_meta_key(repo_name: str, family: str) -> str:
    resolved_family = str(family or "").strip().upper()
    if resolved_family not in {"T", "C"}:
        raise ValueError(f"Unsupported workflow sequence floor family: {family!r}")
    return f"{_WORKFLOW_SEQUENCE_FLOOR_META_PREFIX}:{repo_name}:{resolved_family}"


def _workflow_sequence_floor(conn, repo_name: str, family: str) -> int:
    key = _workflow_sequence_floor_meta_key(repo_name, family)
    row = conn.execute("select value from control_meta where key = ?", (key,)).fetchone()
    if row is None:
        return 0
    try:
        return max(int(str(row["value"] or "0").strip() or "0"), 0)
    except ValueError:
        return 0


def _set_workflow_sequence_floor(conn, repo_name: str, family: str, sequence: int) -> int:
    resolved_sequence = max(int(sequence), 0)
    current = _workflow_sequence_floor(conn, repo_name, family)
    if resolved_sequence <= current:
        return current
    conn.execute(
        "insert or replace into control_meta(key, value) values (?, ?)",
        (_workflow_sequence_floor_meta_key(repo_name, family), str(resolved_sequence)),
    )
    return resolved_sequence


def get_workflow_sequence_floor(ctx: RepoContext, repo_name: str, family: str) -> int:
    conn = _connect_control(ctx)
    try:
        return _workflow_sequence_floor(conn, repo_name, family)
    finally:
        conn.close()


def _next_workflow_sequence(conn, table_name: str, repo_name: str, column_name: str) -> int:
    row = conn.execute(
        f"select max({column_name}) as max_sequence from {table_name} where repo_name = ?",
        (repo_name,),
    ).fetchone()
    return int(row["max_sequence"] or 0) + 1


def _workflow_identity_exists(conn, table_name: str, id_column: str, value: str) -> bool:
    row = conn.execute(f"select 1 from {table_name} where {id_column} = ?", (value,)).fetchone()
    return row is not None


def _workflow_alias_exists(conn, table_name: str, alias_column: str, value: str) -> bool:
    row = conn.execute(f"select 1 from {table_name} where {alias_column} = ?", (value,)).fetchone()
    return row is not None


def allocate_workflow_task_identity(ctx: RepoContext, repo_name: str, namespace_prefix: str | None = None) -> dict:
    conn = _connect_control(ctx)
    task_seq = max(
        _next_workflow_sequence(conn, "workflow_tasks", repo_name, "task_seq") - 1,
        _workflow_sequence_floor(conn, repo_name, "T"),
    ) + 1
    while True:
        task_id = generate_namespaced_sequence_id("T", task_seq, namespace_prefix)
        if not _workflow_identity_exists(conn, "workflow_tasks", "task_id", task_id) and not _workflow_alias_exists(
            conn,
            "workflow_task_aliases",
            "alias_task_id",
            task_id,
        ):
            conn.close()
            return {"task_id": task_id, "task_seq": task_seq}
        task_seq += 1


def allocate_workflow_change_identity(ctx: RepoContext, repo_name: str, namespace_prefix: str | None = None) -> dict:
    conn = _connect_control(ctx)
    change_seq = max(
        _next_workflow_sequence(conn, "workflow_changes", repo_name, "change_seq") - 1,
        _workflow_sequence_floor(conn, repo_name, "C"),
    ) + 1
    while True:
        change_id = generate_namespaced_sequence_id("C", change_seq, namespace_prefix)
        if not _workflow_identity_exists(conn, "workflow_changes", "change_id", change_id) and not _workflow_alias_exists(
            conn,
            "workflow_change_aliases",
            "alias_change_id",
            change_id,
        ):
            conn.close()
            return {"change_id": change_id, "change_seq": change_seq}
        change_seq += 1


def _normalize_task_identity_metadata(task_id: str, task_seq: int | None, identity_source: str | None) -> tuple[int | None, str]:
    resolved_task_seq = task_seq if task_seq is not None else _workflow_sequence_from_id(task_id, family="T")
    if resolved_task_seq is not None:
        return resolved_task_seq, LOCAL_IDENTITY_SOURCE_SEQUENCE
    return None, LOCAL_IDENTITY_SOURCE_LEGACY


def _normalize_change_identity_metadata(change_id: str, change_seq: int | None, identity_source: str | None) -> tuple[int | None, str]:
    resolved_change_seq = change_seq if change_seq is not None else _workflow_sequence_from_id(change_id, family="C")
    if resolved_change_seq is not None:
        return resolved_change_seq, LOCAL_IDENTITY_SOURCE_SEQUENCE
    return None, LOCAL_IDENTITY_SOURCE_LEGACY


def _ensure_local_task_change_identity_schema(conn) -> None:
    _ensure_column(conn, "workflow_tasks", "task_seq", "integer")
    _ensure_column(conn, "workflow_tasks", "identity_source", f"text not null default '{LOCAL_IDENTITY_SOURCE_SEQUENCE}'")
    _ensure_column(conn, "workflow_tasks", "published_remote_name", "text")
    _ensure_column(conn, "workflow_tasks", "published_task_id", "text")
    _ensure_column(conn, "workflow_changes", "change_seq", "integer")
    _ensure_column(conn, "workflow_changes", "identity_source", f"text not null default '{LOCAL_IDENTITY_SOURCE_SEQUENCE}'")
    _ensure_column(conn, "workflow_changes", "published_remote_name", "text")
    _ensure_column(conn, "workflow_changes", "published_change_id", "text")
    conn.executescript(
        """
        create table if not exists workflow_task_aliases (
            alias_task_id text primary key,
            task_id text not null,
            alias_kind text not null,
            created_at text not null,
            foreign key(task_id) references workflow_tasks(task_id)
        );
        create index if not exists idx_workflow_task_aliases_task_id
        on workflow_task_aliases(task_id);
        create index if not exists idx_workflow_tasks_repo_name_task_seq
        on workflow_tasks(repo_name, task_seq);
        create unique index if not exists uq_workflow_tasks_repo_name_task_seq
        on workflow_tasks(repo_name, task_seq)
        where task_seq is not null;
        create table if not exists workflow_change_aliases (
            alias_change_id text primary key,
            change_id text not null,
            alias_kind text not null,
            created_at text not null,
            foreign key(change_id) references workflow_changes(change_id)
        );
        create index if not exists idx_workflow_change_aliases_change_id
        on workflow_change_aliases(change_id);
        create index if not exists idx_workflow_changes_repo_name_change_seq
        on workflow_changes(repo_name, change_seq);
        create unique index if not exists uq_workflow_changes_repo_name_change_seq
        on workflow_changes(repo_name, change_seq)
        where change_seq is not null;
        """
    )
    _backfill_workflow_task_identity_metadata(conn)
    _backfill_workflow_change_identity_metadata(conn)


def _backfill_workflow_task_identity_metadata(conn) -> None:
    rows = conn.execute(
        """
        select task_id, task_seq, identity_source, repo_name, publication_state, published_task_id
        from workflow_tasks
        """
    ).fetchall()
    sequence_floors: dict[str, int] = {}
    for row in rows:
        assignments: list[str] = []
        params: list[object] = []
        task_seq = row["task_seq"]
        parsed_seq = _workflow_sequence_from_id(row["task_id"], family="T")
        if task_seq is None and parsed_seq is not None:
            assignments.append("task_seq = ?")
            params.append(parsed_seq)
        identity_source = str(row["identity_source"] or "").strip() or LOCAL_IDENTITY_SOURCE_SEQUENCE
        if parsed_seq is not None and identity_source != LOCAL_IDENTITY_SOURCE_SEQUENCE:
            assignments.append("identity_source = ?")
            params.append(LOCAL_IDENTITY_SOURCE_SEQUENCE)
        elif parsed_seq is None and identity_source != LOCAL_IDENTITY_SOURCE_LEGACY:
            assignments.append("identity_source = ?")
            params.append(LOCAL_IDENTITY_SOURCE_LEGACY)
        if (
            str(row["publication_state"] or "").strip() == "published"
            and str(row["published_task_id"] or "").strip() == ""
        ):
            assignments.append("published_task_id = ?")
            params.append(str(row["task_id"]))
        repo_name = str(row["repo_name"] or "").strip()
        if repo_name:
            published_value = str(row["published_task_id"] or "").strip()
            if str(row["publication_state"] or "").strip() == "published" and not published_value:
                published_value = str(row["task_id"] or "").strip()
            known_sequences = [
                seq
                for seq in (
                    task_seq,
                    parsed_seq,
                    _workflow_sequence_from_id(published_value, family="T"),
                )
                if seq is not None
            ]
            if known_sequences:
                sequence_floors[repo_name] = max(sequence_floors.get(repo_name, 0), max(known_sequences))
        if not assignments:
            continue
        params.append(str(row["task_id"]))
        conn.execute(f"update workflow_tasks set {', '.join(assignments)} where task_id = ?", tuple(params))
    for repo_name, sequence in sequence_floors.items():
        _set_workflow_sequence_floor(conn, repo_name, "T", sequence)


def _backfill_workflow_change_identity_metadata(conn) -> None:
    rows = conn.execute(
        """
        select change_id, change_seq, identity_source, repo_name, publication_state, published_change_id
        from workflow_changes
        """
    ).fetchall()
    sequence_floors: dict[str, int] = {}
    for row in rows:
        assignments: list[str] = []
        params: list[object] = []
        change_seq = row["change_seq"]
        parsed_seq = _workflow_sequence_from_id(row["change_id"], family="C")
        if change_seq is None and parsed_seq is not None:
            assignments.append("change_seq = ?")
            params.append(parsed_seq)
        identity_source = str(row["identity_source"] or "").strip() or LOCAL_IDENTITY_SOURCE_SEQUENCE
        if parsed_seq is not None and identity_source != LOCAL_IDENTITY_SOURCE_SEQUENCE:
            assignments.append("identity_source = ?")
            params.append(LOCAL_IDENTITY_SOURCE_SEQUENCE)
        elif parsed_seq is None and identity_source != LOCAL_IDENTITY_SOURCE_LEGACY:
            assignments.append("identity_source = ?")
            params.append(LOCAL_IDENTITY_SOURCE_LEGACY)
        if (
            str(row["publication_state"] or "").strip() == "published"
            and str(row["published_change_id"] or "").strip() == ""
        ):
            assignments.append("published_change_id = ?")
            params.append(str(row["change_id"]))
        repo_name = str(row["repo_name"] or "").strip()
        if repo_name:
            published_value = str(row["published_change_id"] or "").strip()
            if str(row["publication_state"] or "").strip() == "published" and not published_value:
                published_value = str(row["change_id"] or "").strip()
            known_sequences = [
                seq
                for seq in (
                    change_seq,
                    parsed_seq,
                    _workflow_sequence_from_id(published_value, family="C"),
                )
                if seq is not None
            ]
            if known_sequences:
                sequence_floors[repo_name] = max(sequence_floors.get(repo_name, 0), max(known_sequences))
        if not assignments:
            continue
        params.append(str(row["change_id"]))
        conn.execute(f"update workflow_changes set {', '.join(assignments)} where change_id = ?", tuple(params))
    for repo_name, sequence in sequence_floors.items():
        _set_workflow_sequence_floor(conn, repo_name, "C", sequence)


def _resolve_workflow_task_id(conn, task_id: str) -> str:
    resolved = str(task_id or "").strip()
    if not resolved:
        raise KeyError("Task id is required.")
    row = conn.execute("select task_id from workflow_tasks where task_id = ?", (resolved,)).fetchone()
    if row is not None:
        return str(row["task_id"])
    row = conn.execute("select task_id from workflow_task_aliases where alias_task_id = ?", (resolved,)).fetchone()
    if row is not None:
        return str(row["task_id"])
    raise KeyError(f"Unknown local task: {task_id}")


def _resolve_workflow_change_id(conn, change_id: str) -> str:
    resolved = str(change_id or "").strip()
    if not resolved:
        raise KeyError("Change id is required.")
    row = conn.execute("select change_id from workflow_changes where change_id = ?", (resolved,)).fetchone()
    if row is not None:
        return str(row["change_id"])
    row = conn.execute("select change_id from workflow_change_aliases where alias_change_id = ?", (resolved,)).fetchone()
    if row is not None:
        return str(row["change_id"])
    raise KeyError(f"Unknown local change: {change_id}")


def _register_workflow_task_alias(conn, alias_task_id: str, task_id: str, *, alias_kind: str) -> None:
    canonical_task_id = _resolve_workflow_task_id(conn, task_id)
    resolved_alias = str(alias_task_id or "").strip()
    if not resolved_alias or resolved_alias == canonical_task_id:
        return
    direct_task = conn.execute("select task_id from workflow_tasks where task_id = ?", (resolved_alias,)).fetchone()
    if direct_task is not None and str(direct_task["task_id"]) != canonical_task_id:
        if alias_kind in _NON_BLOCKING_REMOTE_ALIAS_KINDS:
            return
        raise ValueError(f"Task alias {resolved_alias} is already used by task {direct_task['task_id']}.")
    existing = conn.execute(
        "select task_id from workflow_task_aliases where alias_task_id = ?",
        (resolved_alias,),
    ).fetchone()
    if existing is not None:
        if str(existing["task_id"]) != canonical_task_id:
            if alias_kind in _NON_BLOCKING_REMOTE_ALIAS_KINDS:
                return
            raise ValueError(f"Task alias {resolved_alias} is already mapped to task {existing['task_id']}.")
        return
    conn.execute(
        """
        insert into workflow_task_aliases(alias_task_id, task_id, alias_kind, created_at)
        values (?, ?, ?, ?)
        """,
        (resolved_alias, canonical_task_id, alias_kind, utc_now()),
    )


def _register_workflow_change_alias(conn, alias_change_id: str, change_id: str, *, alias_kind: str) -> None:
    canonical_change_id = _resolve_workflow_change_id(conn, change_id)
    resolved_alias = str(alias_change_id or "").strip()
    if not resolved_alias or resolved_alias == canonical_change_id:
        return
    direct_change = conn.execute("select change_id from workflow_changes where change_id = ?", (resolved_alias,)).fetchone()
    if direct_change is not None and str(direct_change["change_id"]) != canonical_change_id:
        if alias_kind in _NON_BLOCKING_REMOTE_ALIAS_KINDS:
            return
        raise ValueError(f"Change alias {resolved_alias} is already used by change {direct_change['change_id']}.")
    existing = conn.execute(
        "select change_id from workflow_change_aliases where alias_change_id = ?",
        (resolved_alias,),
    ).fetchone()
    if existing is not None:
        if str(existing["change_id"]) != canonical_change_id:
            if alias_kind in _NON_BLOCKING_REMOTE_ALIAS_KINDS:
                return
            raise ValueError(f"Change alias {resolved_alias} is already mapped to change {existing['change_id']}.")
        return
    conn.execute(
        """
        insert into workflow_change_aliases(alias_change_id, change_id, alias_kind, created_at)
        values (?, ?, ?, ?)
        """,
        (resolved_alias, canonical_change_id, alias_kind, utc_now()),
    )
