from __future__ import annotations

from typing import Any

from ait_protocol.common import generate_namespaced_sequence_id, utc_now

from ..server_content_repo_lines import repository_exists
from ..server_control import connect, record_event
from ..server_paths import ServerContext
from .repo_scoped_keys import _next_repo_sequence, _repo_scope_predicate
from .repo_ops import _repo_id, _repo_id_namespace_prefix


def _legacy_server_store_module():
    from .. import server_store as legacy_server_store

    return legacy_server_store


def _refresh_stack_state(*args, **kwargs):
    return _legacy_server_store_module()._refresh_stack_state(*args, **kwargs)


def create_stack(ctx: ServerContext, repo_name: str, title: str, change_ids: list[str] | None = None, landing_policy: str = "ordered") -> dict:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    change_ids = list(dict.fromkeys(change_ids or []))
    repo_id = _repo_id(ctx, repo_name)
    with connect(ctx) as conn:
        for change_id in change_ids:
            row = conn.execute("select 1 from changes where change_id = ? and " + _repo_scope_predicate(), (change_id, repo_id, repo_name)).fetchone()
            if row is None:
                raise KeyError(f"Unknown change {change_id} for repository {repo_name}")
        stack_seq = _next_repo_sequence(conn, "stacks", repo_id, "stack_seq")
        stack_id = generate_namespaced_sequence_id("SK", stack_seq, _repo_id_namespace_prefix(ctx, repo_name))
        now = utc_now()
        conn.execute(
            "insert into stacks(stack_id, repo_name, repo_id, stack_seq, title, landing_policy, status, created_at, updated_at) values (?, ?, ?, ?, ?, ?, 'active', ?, ?)",
            (stack_id, repo_name, repo_id, stack_seq, title, landing_policy, now, now),
        )
        for idx, change_id in enumerate(change_ids, start=1):
            conn.execute(
                "insert into stack_changes(repo_id, stack_id, change_id, position) values (?, ?, ?, ?)",
                (repo_id, stack_id, change_id, idx),
            )
        _refresh_stack_state(conn, stack_id)
        record_event(conn, "stack.created", "stack", stack_id, {"repo_name": repo_name, "title": title, "change_ids": change_ids, "landing_policy": landing_policy})
        conn.commit()
    return get_stack(ctx, stack_id)



def list_stacks(ctx: ServerContext, repo_name: str) -> list[dict]:
    if not repository_exists(ctx, repo_name):
        raise KeyError(f"Unknown repository: {repo_name}")
    repo_id = _repo_id(ctx, repo_name)
    with connect(ctx) as conn:
        rows = [dict(r) for r in conn.execute("select * from stacks where " + _repo_scope_predicate() + " order by updated_at desc", (repo_id, repo_name)).fetchall()]
        for row in rows:
            row["change_ids"] = [r["change_id"] for r in conn.execute("select change_id from stack_changes where stack_id = ? order by position asc", (row["stack_id"],)).fetchall()]
    return rows



def get_stack(ctx: ServerContext, stack_id: str) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select * from stacks where stack_id = ?", (stack_id,)).fetchone()
        if row is None:
            raise KeyError(f"Unknown stack: {stack_id}")
        out = dict(row)
        out["change_ids"] = [r["change_id"] for r in conn.execute("select change_id from stack_changes where stack_id = ? order by position asc", (stack_id,)).fetchall()]
    return out



def update_stack(ctx: ServerContext, stack_id: str, title: str | None = None, landing_policy: str | None = None, status: str | None = None) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select * from stacks where stack_id = ?", (stack_id,)).fetchone()
        if row is None:
            raise KeyError(f"Unknown stack: {stack_id}")
        updates = []
        params: list[Any] = []
        if title is not None:
            updates.append("title = ?")
            params.append(title)
        if landing_policy is not None:
            updates.append("landing_policy = ?")
            params.append(landing_policy)
        if status is not None:
            if status not in {"active", "blocked", "ready_to_land", "landed", "archived"}:
                raise ValueError(f"Invalid stack status: {status}")
            updates.append("status = ?")
            params.append(status)
        if updates:
            params.extend([utc_now(), stack_id])
            conn.execute(f"update stacks set {', '.join(updates)}, updated_at = ? where stack_id = ?", params)
        if status != "archived":
            _refresh_stack_state(conn, stack_id)
        record_event(conn, "stack.updated", "stack", stack_id, {"title": title, "landing_policy": landing_policy, "status": status})
        conn.commit()
    return get_stack(ctx, stack_id)



def add_change_to_stack(ctx: ServerContext, stack_id: str, change_id: str, position: int | None = None) -> dict:
    with connect(ctx) as conn:
        stack = conn.execute("select * from stacks where stack_id = ?", (stack_id,)).fetchone()
        if stack is None:
            raise KeyError(f"Unknown stack: {stack_id}")
        change = conn.execute("select * from changes where change_id = ?", (change_id,)).fetchone()
        if change is None or change["repo_name"] != stack["repo_name"]:
            raise KeyError(f"Change {change_id} does not belong to stack repository {stack['repo_name']}")
        existing = conn.execute("select 1 from stack_changes where stack_id = ? and change_id = ?", (stack_id, change_id)).fetchone()
        if existing is not None:
            raise ValueError(f"Change {change_id} is already in stack {stack_id}")
        count = conn.execute("select count(*) as c from stack_changes where stack_id = ?", (stack_id,)).fetchone()["c"]
        insert_position = position or (count + 1)
        if insert_position < 1 or insert_position > count + 1:
            raise ValueError(f"Invalid stack position: {insert_position}")
        conn.execute("update stack_changes set position = position + 1 where stack_id = ? and position >= ?", (stack_id, insert_position))
        conn.execute(
            "insert into stack_changes(repo_id, stack_id, change_id, position) values (?, ?, ?, ?)",
            (stack["repo_id"] or _repo_id(ctx, stack["repo_name"]), stack_id, change_id, insert_position),
        )
        _refresh_stack_state(conn, stack_id)
        record_event(conn, "stack.updated", "stack", stack_id, {"action": "add_change", "change_id": change_id, "position": insert_position})
        conn.commit()
    return get_stack(ctx, stack_id)



def remove_change_from_stack(ctx: ServerContext, stack_id: str, change_id: str) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select position from stack_changes where stack_id = ? and change_id = ?", (stack_id, change_id)).fetchone()
        if row is None:
            raise KeyError(f"Change {change_id} is not in stack {stack_id}")
        position = row["position"]
        conn.execute("delete from stack_changes where stack_id = ? and change_id = ?", (stack_id, change_id))
        conn.execute("update stack_changes set position = position - 1 where stack_id = ? and position > ?", (stack_id, position))
        _refresh_stack_state(conn, stack_id)
        record_event(conn, "stack.updated", "stack", stack_id, {"action": "remove_change", "change_id": change_id})
        conn.commit()
    return get_stack(ctx, stack_id)



def reorder_stack_change(ctx: ServerContext, stack_id: str, change_id: str, position: int) -> dict:
    with connect(ctx) as conn:
        row = conn.execute("select position from stack_changes where stack_id = ? and change_id = ?", (stack_id, change_id)).fetchone()
        if row is None:
            raise KeyError(f"Change {change_id} is not in stack {stack_id}")
        current_pos = row["position"]
        count = conn.execute("select count(*) as c from stack_changes where stack_id = ?", (stack_id,)).fetchone()["c"]
        if position < 1 or position > count:
            raise ValueError(f"Invalid stack position: {position}")
        if position == current_pos:
            return get_stack(ctx, stack_id)
        conn.execute("update stack_changes set position = 0 where stack_id = ? and change_id = ?", (stack_id, change_id))
        if position < current_pos:
            conn.execute(
                "update stack_changes set position = position + 1 where stack_id = ? and position >= ? and position < ?",
                (stack_id, position, current_pos),
            )
        else:
            conn.execute(
                "update stack_changes set position = position - 1 where stack_id = ? and position > ? and position <= ?",
                (stack_id, current_pos, position),
            )
        conn.execute("update stack_changes set position = ? where stack_id = ? and change_id = ?", (position, stack_id, change_id))
        _refresh_stack_state(conn, stack_id)
        record_event(conn, "stack.updated", "stack", stack_id, {"action": "reorder_change", "change_id": change_id, "position": position})
        conn.commit()
    return get_stack(ctx, stack_id)



def get_stack_graph(ctx: ServerContext, stack_id: str) -> dict:
    with connect(ctx) as conn:
        stack = conn.execute("select * from stacks where stack_id = ?", (stack_id,)).fetchone()
        if stack is None:
            raise KeyError(f"Unknown stack: {stack_id}")
        members = [
            dict(r)
            for r in conn.execute(
                """
                select sc.position, c.change_id, c.status, c.current_patchset_number
                from stack_changes sc
                join changes c on c.change_id = sc.change_id
                where sc.stack_id = ?
                order by sc.position asc
                """,
                (stack_id,),
            ).fetchall()
        ]
    nodes = [
        {
            "change_id": row["change_id"],
            "position": row["position"],
            "status": row["status"],
            "patchset_number": row["current_patchset_number"],
        }
        for row in members
    ]
    edges = [
        {
            "from_change_id": members[idx]["change_id"],
            "to_change_id": members[idx + 1]["change_id"],
            "kind": "ordered",
        }
        for idx in range(len(members) - 1)
    ]
    return {
        "stack_id": stack_id,
        "title": stack["title"],
        "landing_policy": stack["landing_policy"],
        "status": stack["status"],
        "nodes": nodes,
        "edges": edges,
    }
