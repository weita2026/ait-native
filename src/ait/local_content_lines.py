from __future__ import annotations

from pathlib import Path
from typing import Optional

from ait_protocol.common import connect_sqlite, encode_ref_name, utc_now

from .repo_paths import RepoContext


def _ref_path(ctx: RepoContext, line_name: str) -> Path:
    return ctx.ref_dir / encode_ref_name(line_name)


def read_ref(ctx: RepoContext, line_name: str) -> str | None:
    path = _ref_path(ctx, line_name)
    if not path.exists():
        return None
    text = path.read_text(encoding="utf-8").strip()
    return text or None


def write_ref(ctx: RepoContext, line_name: str, snapshot_id: str | None) -> None:
    path = _ref_path(ctx, line_name)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text((snapshot_id or "") + "\n", encoding="utf-8")


def get_line(ctx: RepoContext, line_name: str) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    row = conn.execute("select * from lines where line_name = ?", (line_name,)).fetchone()
    conn.close()
    if row is None:
        raise KeyError(f"Unknown line: {line_name}")
    out = dict(row)
    out["status"] = out.get("status") or "active"
    out["head_snapshot_id"] = read_ref(ctx, line_name)
    return out


def list_lines(ctx: RepoContext) -> list[dict]:
    conn = connect_sqlite(ctx.content_db_path)
    rows = [dict(r) for r in conn.execute("select * from lines order by line_name")]
    conn.close()
    for row in rows:
        row["status"] = row.get("status") or "active"
        row["head_snapshot_id"] = read_ref(ctx, row["line_name"])
    return rows


def create_line(ctx: RepoContext, line_name: str, from_snapshot_id: Optional[str] = None) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    now = utc_now()
    conn.execute(
        "insert into lines(line_name, status, archived_at, created_at, updated_at) values (?, 'active', null, ?, ?)",
        (line_name, now, now),
    )
    conn.commit()
    conn.close()
    write_ref(ctx, line_name, from_snapshot_id)
    return get_line(ctx, line_name)


def set_line_head(ctx: RepoContext, line_name: str, snapshot_id: Optional[str]) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    row = conn.execute("select * from lines where line_name = ?", (line_name,)).fetchone()
    if row is None:
        now = utc_now()
        conn.execute(
            "insert into lines(line_name, status, archived_at, created_at, updated_at) values (?, 'active', null, ?, ?)",
            (line_name, now, now),
        )
    else:
        if (row["status"] or "active") == "archived":
            conn.close()
            raise ValueError(f"Line {line_name} is archived and cannot move")
        conn.execute("update lines set updated_at = ? where line_name = ?", (utc_now(), line_name))
    conn.commit()
    conn.close()
    write_ref(ctx, line_name, snapshot_id)
    return get_line(ctx, line_name)


def archive_line(ctx: RepoContext, line_name: str) -> dict:
    conn = connect_sqlite(ctx.content_db_path)
    row = conn.execute("select * from lines where line_name = ?", (line_name,)).fetchone()
    if row is None:
        conn.close()
        raise KeyError(f"Unknown line: {line_name}")
    if (row["status"] or "active") == "archived":
        conn.close()
        return get_line(ctx, line_name)
    now = utc_now()
    conn.execute(
        "update lines set status = 'archived', archived_at = ?, updated_at = ? where line_name = ?",
        (now, now, line_name),
    )
    conn.commit()
    conn.close()
    return get_line(ctx, line_name)
