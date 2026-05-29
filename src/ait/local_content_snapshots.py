from __future__ import annotations

import secrets
import sqlite3
import time
from pathlib import Path
from typing import Any, Callable, Iterable, Optional

from ait_protocol.common import connect_sqlite, normalize_optional_text, utc_now
from ait_storage.revision_trees import build_snapshot_id, build_tree_records

from .local_content_lines import read_ref, write_ref
from .local_content_projection import (
    _effective_workspace_ignore_rules,
    _filter_snapshot_file_map_for_workspace,
    _filter_workspace_state_for_workspace,
    _is_lineage_only_markdown_artifact_path,
    _path_is_projected_out_for_task_worktree,
    _path_is_projected_out_for_workspace,
)
from .local_content_pack_runtime import (
    _blob_bytes_by_id,
    _blob_path,
    _blob_row,
    _insert_blob_record,
    _insert_tree_records,
    _manifest_path_for_tree,
    _parent_delta_candidates,
    _snapshot_row,
    _tree_entry_blob_size_sql,
    _write_packed_blobs,
    _write_tree_pack,
)
from .local_content_workspace import (
    WorkspaceIgnoreRule,
    _normalize_workspace_restore_path,
    _snapshot_workspace_ignore_rules,
    _workspace_digest_state,
    _workspace_ignore_policy_for_rules,
    _workspace_path_is_ignored,
    _workspace_state,
    _workspace_visible_files,
)
from .repo_paths import RepoContext


def _elapsed_ms(start: float, end: float | None = None) -> float:
    finished = time.perf_counter() if end is None else end
    return round((finished - start) * 1000.0, 3)


def _canonical_snapshot_metadata(
    repo_name: str,
    line_name: str,
    parent_snapshot_id: str | None,
    message: str | None,
    file_entries: list[dict[str, Any]],
    *,
    snapshot_kind: str = "line",
) -> dict[str, Any]:
    root_tree_id, tree_rows, tree_entry_rows = build_tree_records(file_entries)
    snapshot_id, revision_hash = build_snapshot_id(
        repo_name=repo_name,
        line_name=line_name,
        parent_snapshot_id=parent_snapshot_id,
        message=message,
        root_tree_id=root_tree_id,
        snapshot_kind=snapshot_kind,
    )
    return {
        "snapshot_id": snapshot_id,
        "revision_hash": revision_hash,
        "root_tree_id": root_tree_id,
        "manifest_path": f"trees/{root_tree_id}",
        "tree_rows": tree_rows,
        "tree_entry_rows": tree_entry_rows,
    }


def snapshot_exists(ctx: RepoContext, snapshot_id: str) -> bool:
    conn = connect_sqlite(ctx.content_db_path)
    row = _snapshot_row(conn, snapshot_id)
    conn.close()
    return row is not None


def create_snapshot(
    ctx: RepoContext,
    repo_name: str,
    line_name: str,
    message: Optional[str],
    *,
    parent_snapshot_id: str | None = None,
    update_line_ref: bool = True,
    snapshot_kind: str = "line",
    touch_line: bool = True,
) -> dict:
    normalized_snapshot_kind = str(snapshot_kind or "line").strip().lower() or "line"
    if normalized_snapshot_kind not in {"line", "stash"}:
        raise ValueError(f"Unsupported snapshot kind: {snapshot_kind}")
    total_started = time.perf_counter()
    conn = connect_sqlite(ctx.content_db_path)
    line_row = conn.execute("select * from lines where line_name = ?", (line_name,)).fetchone()
    if line_row is None:
        conn.close()
        raise KeyError(f"Current line does not exist: {line_name}")
    if (line_row["status"] or "active") == "archived":
        conn.close()
        raise ValueError(f"Current line {line_name} is archived and cannot create snapshots")
    resolved_parent_snapshot_id = parent_snapshot_id if parent_snapshot_id is not None else read_ref(ctx, line_name)

    phase_timings_ms: dict[str, Any] = {}
    entries: list[dict] = []
    new_blob_rows: list[dict[str, Any]] = []
    total_bytes = 0
    effective_ignore_rules = _effective_workspace_ignore_rules(ctx)
    visible_paths = _workspace_visible_files(ctx.root, ignore_rules=effective_ignore_rules, phase_timings_ms=phase_timings_ms)

    projection_started = time.perf_counter()
    materialized_paths: list[tuple[Path, str]] = []
    for path in visible_paths:
        rel = path.relative_to(ctx.root).as_posix()
        if _path_is_projected_out_for_workspace(ctx, rel):
            continue
        materialized_paths.append((path, rel))
    phase_timings_ms["workspace_projection_filter"] = _elapsed_ms(projection_started)

    materialized_state = _workspace_digest_state(
        ctx.root,
        materialized_paths,
        include_data=True,
        phase_timings_ms=phase_timings_ms,
    )
    for rel in sorted(materialized_state):
        path_state = materialized_state[rel]
        digest = str(path_state["sha256"])
        blob_id = f"BLB-{digest[:20]}"
        size = int(path_state["size_bytes"])
        total_bytes += size
        blob_storage = _blob_path(ctx, blob_id)
        existing = _blob_row(conn, blob_id)
        data = path_state.get("data")
        if existing is None:
            if data is None:
                data = Path(path_state["abs_path"]).read_bytes()
            new_blob_rows.append(
                {
                    "blob_id": blob_id,
                    "sha256": digest,
                    "storage_path": str(blob_storage.relative_to(ctx.root)),
                    "size_bytes": size,
                    "data": data,
                    "entry_name": f"blobs/{blob_id}",
                    "path_hint": rel,
                }
            )
        entries.append(
            {
                "path": rel,
                "blob_id": blob_id,
                "size_bytes": size,
                "mode": str(path_state["mode"]),
                "sha256": digest,
            }
        )

    projected_parent_started = time.perf_counter()
    if resolved_parent_snapshot_id is not None:
        existing_paths = {entry["path"] for entry in entries}
        parent_snapshot_files = _snapshot_file_map(conn, resolved_parent_snapshot_id)
        for rel, parent_entry in sorted(parent_snapshot_files.items()):
            if (
                rel in existing_paths
                or not _path_is_projected_out_for_task_worktree(ctx, rel)
                or _is_lineage_only_markdown_artifact_path(rel)
            ):
                continue
            blob = _blob_row(conn, parent_entry["blob_id"])
            if blob is None:
                conn.close()
                raise KeyError(f"Unknown blob: {parent_entry['blob_id']}")
            size_bytes = int(parent_entry["size_bytes"] or 0)
            total_bytes += size_bytes
            entries.append(
                {
                    "path": rel,
                    "blob_id": parent_entry["blob_id"],
                    "size_bytes": size_bytes,
                    "mode": parent_entry["mode"],
                    "sha256": blob["sha256"],
                }
            )
    phase_timings_ms["projected_parent_reuse"] = _elapsed_ms(projected_parent_started)

    snapshot_meta = _canonical_snapshot_metadata(
        repo_name,
        line_name,
        resolved_parent_snapshot_id,
        message,
        entries,
        snapshot_kind=normalized_snapshot_kind,
    )
    snapshot_id = snapshot_meta["snapshot_id"]
    created_at = utc_now()

    blob_pack_ms = 0.0
    if new_blob_rows:
        initial_by_path = _parent_delta_candidates(
            ctx,
            conn,
            resolved_parent_snapshot_id,
            {str(row.get("path_hint") or "") for row in new_blob_rows if row.get("path_hint")},
        )
        blob_pack_started = time.perf_counter()
        pack_id, members_by_blob_id = _write_packed_blobs(
            ctx,
            conn,
            snapshot_id,
            created_at,
            new_blob_rows,
            initial_by_path=initial_by_path,
        )
        blob_pack_ms = _elapsed_ms(blob_pack_started)
        for row in new_blob_rows:
            member = members_by_blob_id[row["blob_id"]]
            entry_type = member.get("entry_type", "full")
            _insert_blob_record(
                conn,
                blob_id=row["blob_id"],
                sha256=row["sha256"],
                storage_path=row["storage_path"],
                size_bytes=row["size_bytes"],
                pack_id=pack_id,
                pack_entry_type=entry_type,
                pack_base_blob_id=member.get("base_blob_id"),
                pack_chain_depth=int(member.get("chain_depth", 0) or 0),
                pruned_at=None,
                created_at=created_at,
                pack_entry_name=None,
                packed_at=None,
            )

    tree_record_stage_started = time.perf_counter()
    _insert_tree_records(conn, snapshot_meta["tree_rows"], snapshot_meta["tree_entry_rows"], created_at)
    phase_timings_ms["tree_record_stage"] = _elapsed_ms(tree_record_stage_started)

    tree_pack_started = time.perf_counter()
    _write_tree_pack(
        ctx,
        conn,
        [str(row["tree_id"]) for row in snapshot_meta["tree_rows"]],
        created_at=created_at,
        seed_hint=f"{snapshot_id}|{snapshot_meta['root_tree_id']}",
    )
    tree_pack_ms = _elapsed_ms(tree_pack_started)
    phase_timings_ms["pack_archive_write"] = {
        "blob_pack_write": blob_pack_ms,
        "tree_pack_write": tree_pack_ms,
        "total": round(blob_pack_ms + tree_pack_ms, 3),
    }

    metadata_commit_started = time.perf_counter()
    manifest_path = _manifest_path_for_tree(conn, snapshot_meta["root_tree_id"])
    if _snapshot_row(conn, snapshot_id) is None:
        conn.execute(
            """
            insert into snapshots(
                snapshot_id, parent_snapshot_id, root_tree_id, manifest_hash, manifest_path,
                message, line_name, snapshot_kind, file_count, total_bytes, created_at
            ) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                snapshot_id,
                resolved_parent_snapshot_id,
                snapshot_meta["root_tree_id"],
                snapshot_meta["revision_hash"],
                manifest_path,
                message,
                line_name,
                normalized_snapshot_kind,
                len(entries),
                total_bytes,
                created_at,
            ),
        )
    if touch_line:
        conn.execute("update lines set updated_at = ? where line_name = ?", (created_at, line_name))
    conn.commit()
    conn.close()
    if update_line_ref:
        write_ref(ctx, line_name, snapshot_id)
    phase_timings_ms["metadata_commit"] = _elapsed_ms(metadata_commit_started)
    phase_timings_ms["total"] = _elapsed_ms(total_started)

    snapshot = get_snapshot(ctx, snapshot_id)
    snapshot["ignore_policy"] = _workspace_ignore_policy_for_rules(ctx.root, effective_ignore_rules)
    snapshot["phase_timings_ms"] = phase_timings_ms
    return snapshot


def list_snapshots(ctx: RepoContext) -> list[dict]:
    conn = connect_sqlite(ctx.content_db_path)
    rows = [
        dict(r)
        for r in conn.execute(
            "select * from snapshots where coalesce(snapshot_kind, 'line') = 'line' order by created_at desc, rowid desc"
        )
    ]
    conn.close()
    return rows


def _stash_select_sql() -> str:
    return """
        select
            st.stash_id,
            st.snapshot_id,
            st.source_line_name,
            st.base_snapshot_id,
            st.message as stash_message,
            st.workspace_cleared,
            st.created_at as stash_created_at,
            s.parent_snapshot_id,
            s.file_count,
            s.total_bytes,
            s.snapshot_kind,
            s.created_at as snapshot_created_at,
            s.message as snapshot_message
        from stashes st
        join snapshots s on s.snapshot_id = st.snapshot_id
    """


def _stash_view(row: sqlite3.Row | None) -> dict[str, Any]:
    if row is None:
        raise KeyError("Unknown stash")
    payload = dict(row)
    return {
        "stash_id": payload["stash_id"],
        "snapshot_id": payload["snapshot_id"],
        "source_line_name": payload["source_line_name"],
        "base_snapshot_id": payload["base_snapshot_id"],
        "message": payload.get("stash_message")
        if payload.get("stash_message") is not None
        else payload.get("snapshot_message"),
        "workspace_cleared": bool(payload.get("workspace_cleared")),
        "created_at": payload["stash_created_at"],
        "snapshot_created_at": payload["snapshot_created_at"],
        "snapshot_kind": payload.get("snapshot_kind") or "line",
        "parent_snapshot_id": payload.get("parent_snapshot_id"),
        "file_count": int(payload.get("file_count") or 0),
        "total_bytes": int(payload.get("total_bytes") or 0),
    }


def create_stash(
    ctx: RepoContext,
    *,
    snapshot_id: str,
    source_line_name: str,
    base_snapshot_id: str | None,
    message: str | None,
    workspace_cleared: bool,
) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    snapshot_row = _snapshot_row(conn, snapshot_id)
    if snapshot_row is None:
        conn.close()
        raise KeyError(f"Unknown snapshot: {snapshot_id}")
    if (snapshot_row["snapshot_kind"] or "line") != "stash":
        conn.close()
        raise ValueError(f"Snapshot {snapshot_id} is not a stash snapshot.")
    stash_id = f"STH-{secrets.token_hex(10).upper()}"
    created_at = utc_now()
    conn.execute(
        """
        insert into stashes(
            stash_id, snapshot_id, source_line_name, base_snapshot_id, message, workspace_cleared, created_at
        ) values (?, ?, ?, ?, ?, ?, ?)
        """,
        (
            stash_id,
            snapshot_id,
            source_line_name,
            base_snapshot_id,
            message,
            1 if workspace_cleared else 0,
            created_at,
        ),
    )
    row = conn.execute(f"{_stash_select_sql()} where st.stash_id = ?", (stash_id,)).fetchone()
    conn.commit()
    conn.close()
    return _stash_view(row)


def list_stashes(ctx: RepoContext) -> list[dict]:
    conn = connect_sqlite(ctx.content_db_path)
    rows = [
        _stash_view(row)
        for row in conn.execute(f"{_stash_select_sql()} order by st.created_at desc, st.stash_id desc")
    ]
    conn.close()
    return rows


def get_stash(ctx: RepoContext, stash_id: str) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    row = conn.execute(f"{_stash_select_sql()} where st.stash_id = ?", (stash_id,)).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown stash: {stash_id}")
    return _stash_view(row)


def drop_stash(ctx: RepoContext, stash_id: str) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    row = conn.execute(f"{_stash_select_sql()} where st.stash_id = ?", (stash_id,)).fetchone()
    if row is None:
        conn.close()
        raise KeyError(f"Unknown stash: {stash_id}")
    stash = _stash_view(row)
    conn.execute("delete from stashes where stash_id = ?", (stash_id,))
    remaining = conn.execute("select count(*) as c from stashes where snapshot_id = ?", (stash["snapshot_id"],)).fetchone()["c"]
    if remaining == 0:
        conn.execute(
            "delete from snapshots where snapshot_id = ? and coalesce(snapshot_kind, 'line') = 'stash'",
            (stash["snapshot_id"],),
        )
    conn.commit()
    conn.close()
    return {
        **stash,
        "dropped": True,
        "snapshot_deleted": remaining == 0,
    }


def get_snapshot(ctx: RepoContext, snapshot_id: str) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    snap = _snapshot_row(conn, snapshot_id)
    if snap is None:
        conn.close()
        raise KeyError(f"Unknown snapshot: {snapshot_id}")
    files = [
        dict(r)
        for r in conn.execute(
            "select path, blob_id, size_bytes, mode from snapshot_files where snapshot_id = ? order by path",
            (snapshot_id,),
        )
    ]
    conn.close()
    out = dict(snap)
    out["files"] = files
    return out


def _parse_mode_bits(mode: str | None) -> int:
    text = str(mode or "0o644").strip() or "0o644"
    try:
        return int(text, 0)
    except ValueError:
        return int(text, 8)


def _snapshot_file_map(
    conn,
    snapshot_id: str | None,
    *,
    snapshot_row_fn: Callable[[Any, str], Any] = _snapshot_row,
    snapshot_file_map_via_view_fn: Callable[[Any, str], dict[str, dict]] | None = None,
    snapshot_file_map_via_tree_traversal_fn: Callable[[Any, str], dict[str, dict]] | None = None,
) -> dict[str, dict]:
    if snapshot_id is None:
        return {}
    if snapshot_file_map_via_view_fn is None:
        snapshot_file_map_via_view_fn = _snapshot_file_map_via_view
    if snapshot_file_map_via_tree_traversal_fn is None:
        snapshot_file_map_via_tree_traversal_fn = _snapshot_file_map_via_tree_traversal
    snap = snapshot_row_fn(conn, snapshot_id)
    if snap is None:
        raise KeyError(f"Unknown snapshot: {snapshot_id}")
    root_tree_id = normalize_optional_text(snap["root_tree_id"])
    if root_tree_id is not None:
        return snapshot_file_map_via_tree_traversal_fn(conn, root_tree_id)
    return snapshot_file_map_via_view_fn(conn, snapshot_id)


def _snapshot_file_map_via_view(conn, snapshot_id: str) -> dict[str, dict]:
    rows = conn.execute(
        """
        select sf.path, sf.blob_id, sf.size_bytes, sf.mode, b.sha256
        from snapshot_files sf
        join blobs b on b.blob_id = sf.blob_id
        where sf.snapshot_id = ?
        order by sf.path
        """,
        (snapshot_id,),
    ).fetchall()
    return {
        row["path"]: {
            "path": row["path"],
            "blob_id": row["blob_id"],
            "sha256": row["sha256"],
            "size_bytes": row["size_bytes"],
            "mode": row["mode"],
        }
        for row in rows
    }


def _blob_sha256_map(conn, blob_ids: list[str]) -> dict[str, str]:
    if not blob_ids:
        return {}
    rows_by_blob: dict[str, str] = {}
    chunk_size = 400
    for start in range(0, len(blob_ids), chunk_size):
        chunk = blob_ids[start:start + chunk_size]
        placeholders = ", ".join("?" for _ in chunk)
        rows = conn.execute(
            f"select blob_id, sha256 from blobs where blob_id in ({placeholders})",
            tuple(chunk),
        ).fetchall()
        for row in rows:
            blob_id = str(row["blob_id"] or "").strip()
            sha256 = str(row["sha256"] or "").strip()
            if blob_id:
                rows_by_blob[blob_id] = sha256
    return rows_by_blob


def _snapshot_file_map_via_tree_traversal(conn, root_tree_id: str) -> dict[str, dict]:
    tree_rows: list[dict[str, Any]] = []
    stack: list[tuple[str, str]] = [("", root_tree_id)]
    while stack:
        prefix, tree_id = stack.pop()
        size_expr = _tree_entry_blob_size_sql(conn, "te", "b")
        rows = conn.execute(
            f"""
            select
                te.entry_name,
                te.entry_type,
                te.target_id,
                {size_expr} as size_bytes,
                te.mode
            from tree_entries te
            left join blobs b on b.blob_id = te.target_id
            where te.tree_id = ?
            order by te.entry_name asc
            """,
            (tree_id,),
        ).fetchall()
        for row in rows:
            entry_name = str(row["entry_name"] or "")
            path = f"{prefix}{entry_name}"
            entry_type = str(row["entry_type"] or "")
            target_id = str(row["target_id"] or "")
            if entry_type == "blob":
                tree_rows.append(
                    {
                        "path": path,
                        "blob_id": target_id,
                        "size_bytes": row["size_bytes"],
                        "mode": row["mode"],
                    }
                )
            elif entry_type == "tree" and target_id:
                stack.append((path + "/", target_id))

    sha256_by_blob = _blob_sha256_map(conn, sorted({str(row["blob_id"]) for row in tree_rows}))
    return {
        row["path"]: {
            "path": row["path"],
            "blob_id": row["blob_id"],
            "sha256": sha256_by_blob.get(str(row["blob_id"]), ""),
            "size_bytes": row["size_bytes"],
            "mode": row["mode"],
        }
        for row in sorted(tree_rows, key=lambda item: str(item["path"]))
    }


def workspace_delta(
    ctx: RepoContext,
    snapshot_id: str | None,
    *,
    ignore_rules: tuple[WorkspaceIgnoreRule, ...] | None = None,
) -> dict:
    total_started = time.perf_counter()
    conn = connect_sqlite(ctx.content_db_path)
    baseline_read_started = time.perf_counter()
    snapshot_files = _filter_snapshot_file_map_for_workspace(ctx, _snapshot_file_map(conn, snapshot_id))
    conn.close()

    effective_ignore_rules = _effective_workspace_ignore_rules(ctx, ignore_rules)
    snapshot_files = {
        path: snapshot_file
        for path, snapshot_file in snapshot_files.items()
        if not _workspace_path_is_ignored(Path(path), effective_ignore_rules)
    }
    phase_timings_ms: dict[str, Any] = {
        "baseline_snapshot_read": _elapsed_ms(baseline_read_started),
    }
    workspace_files = _workspace_state(
        ctx.root,
        ignore_rules=effective_ignore_rules,
        phase_timings_ms=phase_timings_ms,
    )
    projection_started = time.perf_counter()
    workspace_files = _filter_workspace_state_for_workspace(ctx, workspace_files)
    phase_timings_ms["workspace_projection_filter"] = _elapsed_ms(projection_started)

    compare_started = time.perf_counter()
    modified_paths: list[str] = []
    missing_paths: list[str] = []
    for path, snapshot_file in snapshot_files.items():
        current = workspace_files.pop(path, None)
        if current is None:
            missing_paths.append(path)
            continue
        if current["sha256"] != snapshot_file["sha256"] or current["mode"] != snapshot_file["mode"]:
            modified_paths.append(path)
    untracked_paths = sorted(workspace_files)
    modified_paths = sorted(modified_paths)
    missing_paths = sorted(missing_paths)
    changed_paths = sorted({*modified_paths, *missing_paths, *untracked_paths})
    phase_timings_ms["compare_manifest"] = _elapsed_ms(compare_started)
    phase_timings_ms["total"] = _elapsed_ms(total_started)
    return {
        "snapshot_id": snapshot_id,
        "clean": len(changed_paths) == 0,
        "changed_count": len(changed_paths),
        "changed_paths": changed_paths,
        "modified_paths": modified_paths,
        "missing_paths": missing_paths,
        "untracked_paths": untracked_paths,
        "ignore_policy": _workspace_ignore_policy_for_rules(ctx.root, effective_ignore_rules),
        "phase_timings_ms": phase_timings_ms,
    }


def _prune_empty_parent_dirs(root: Path, path: Path) -> None:
    cur = path.parent
    while cur != root and cur.exists():
        try:
            cur.rmdir()
        except OSError:
            break
        cur = cur.parent


def restore_workspace(
    ctx: RepoContext,
    target_snapshot_id: str | None,
    *,
    baseline_snapshot_id: str | None = None,
    force: bool = False,
    dry_run: bool = False,
) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    target_files = _filter_snapshot_file_map_for_workspace(ctx, _snapshot_file_map(conn, target_snapshot_id))
    snapshot_ignore_rules = _snapshot_workspace_ignore_rules(
        target_snapshot_id,
        target_files,
        lambda blob_id: _blob_bytes_by_id(ctx, conn, blob_id).decode("utf-8", errors="replace"),
    )
    effective_ignore_rules = _effective_workspace_ignore_rules(ctx, snapshot_ignore_rules)
    dirty = workspace_delta(ctx, baseline_snapshot_id, ignore_rules=snapshot_ignore_rules)
    workspace_files = _filter_workspace_state_for_workspace(
        ctx,
        _workspace_state(ctx.root, ignore_rules=effective_ignore_rules),
    )

    write_paths: list[str] = []
    for path, target in target_files.items():
        current = workspace_files.pop(path, None)
        if current is None or current["sha256"] != target["sha256"] or current["mode"] != target["mode"]:
            write_paths.append(path)
    remove_paths = sorted(workspace_files, key=lambda item: (item.count("/"), item), reverse=True)
    result = {
        "target_snapshot_id": target_snapshot_id,
        "baseline_snapshot_id": baseline_snapshot_id,
        "force": force,
        "dry_run": dry_run,
        "applied": False,
        "workspace_dirty": not dirty["clean"],
        "would_overwrite_workspace_changes": not dirty["clean"],
        "dirty_workspace": dirty,
        "plan": {
            "write_count": len(write_paths),
            "remove_count": len(remove_paths),
            "unchanged_count": max(len(target_files) - len(write_paths), 0),
            "write_paths": sorted(write_paths),
            "remove_paths": remove_paths,
        },
    }
    if dirty["changed_count"] and not force and not dry_run:
        sample = ", ".join(dirty["changed_paths"][:5])
        if len(dirty["changed_paths"]) > 5:
            sample += ", ..."
        baseline_label = baseline_snapshot_id or "empty workspace"
        conn.close()
        raise ValueError(f"Workspace has unsaved changes relative to {baseline_label}: {sample}")
    if dry_run:
        conn.close()
        return result

    for rel in remove_paths:
        abs_path = ctx.root / rel
        if abs_path.exists():
            abs_path.unlink()
            _prune_empty_parent_dirs(ctx.root, abs_path)

    for rel in sorted(write_paths):
        target = target_files[rel]
        abs_path = ctx.root / rel
        abs_path.parent.mkdir(parents=True, exist_ok=True)
        if abs_path.exists() and abs_path.is_dir():
            conn.close()
            raise IsADirectoryError(f"Cannot restore file over directory: {rel}")
        data = _blob_bytes_by_id(ctx, conn, target["blob_id"])
        abs_path.write_bytes(data)
        abs_path.chmod(_parse_mode_bits(target["mode"]))

    conn.close()
    result["applied"] = True
    return result


def restore_workspace_paths(
    ctx: RepoContext,
    target_snapshot_id: str | None,
    paths: Iterable[str | Path],
    *,
    baseline_snapshot_id: str | None = None,
    force: bool = False,
    dry_run: bool = False,
) -> dict:
    requested_paths = sorted({_normalize_workspace_restore_path(path) for path in paths})
    conn = connect_sqlite(ctx.content_db_path)
    target_files = _filter_snapshot_file_map_for_workspace(ctx, _snapshot_file_map(conn, target_snapshot_id))
    snapshot_ignore_rules = _snapshot_workspace_ignore_rules(
        target_snapshot_id,
        target_files,
        lambda blob_id: _blob_bytes_by_id(ctx, conn, blob_id).decode("utf-8", errors="replace"),
    )
    effective_ignore_rules = _effective_workspace_ignore_rules(ctx, snapshot_ignore_rules)
    dirty = workspace_delta(ctx, baseline_snapshot_id, ignore_rules=snapshot_ignore_rules)
    workspace_files = _filter_workspace_state_for_workspace(
        ctx,
        _workspace_state(ctx.root, ignore_rules=effective_ignore_rules),
    )

    requested_set = set(requested_paths)
    dirty_selected_paths = sorted(set(dirty["changed_paths"]) & requested_set)
    dirty_outside_paths = sorted(set(dirty["changed_paths"]) - requested_set)

    write_paths: list[str] = []
    remove_paths: list[str] = []
    unchanged_paths: list[str] = []
    for rel in requested_paths:
        target = target_files.get(rel)
        current = workspace_files.get(rel)
        if target is None:
            if current is None:
                unchanged_paths.append(rel)
            else:
                remove_paths.append(rel)
            continue
        if current is None or current["sha256"] != target["sha256"] or current["mode"] != target["mode"]:
            write_paths.append(rel)
        else:
            unchanged_paths.append(rel)

    result = {
        "target_snapshot_id": target_snapshot_id,
        "baseline_snapshot_id": baseline_snapshot_id,
        "force": force,
        "dry_run": dry_run,
        "applied": False,
        "workspace_dirty": not dirty["clean"],
        "would_overwrite_selected_changes": bool(dirty_selected_paths),
        "dirty_workspace": dirty,
        "dirty_selected_paths": dirty_selected_paths,
        "dirty_outside_paths": dirty_outside_paths,
        "plan": {
            "write_count": len(write_paths),
            "remove_count": len(remove_paths),
            "unchanged_count": len(unchanged_paths),
            "requested_paths": requested_paths,
            "write_paths": sorted(write_paths),
            "remove_paths": sorted(remove_paths, key=lambda item: (item.count("/"), item), reverse=True),
            "unchanged_paths": unchanged_paths,
        },
    }
    if dirty_selected_paths and not force and not dry_run:
        sample = ", ".join(dirty_selected_paths[:5])
        if len(dirty_selected_paths) > 5:
            sample += ", ..."
        baseline_label = baseline_snapshot_id or "empty workspace"
        conn.close()
        raise ValueError(f"Selected paths have unsaved changes relative to {baseline_label}: {sample}")
    if dry_run:
        conn.close()
        return result

    for rel in result["plan"]["remove_paths"]:
        abs_path = ctx.root / rel
        if abs_path.exists():
            abs_path.unlink()
            _prune_empty_parent_dirs(ctx.root, abs_path)

    for rel in sorted(write_paths):
        target = target_files[rel]
        abs_path = ctx.root / rel
        abs_path.parent.mkdir(parents=True, exist_ok=True)
        if abs_path.exists() and abs_path.is_dir():
            conn.close()
            raise IsADirectoryError(f"Cannot restore file over directory: {rel}")
        data = _blob_bytes_by_id(ctx, conn, target["blob_id"])
        abs_path.write_bytes(data)
        abs_path.chmod(_parse_mode_bits(target["mode"]))

    conn.close()
    result["applied"] = True
    return result
