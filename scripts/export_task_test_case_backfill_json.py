#!/usr/bin/env python3
"""Export per-task test-case backfill JSON from local workflow + PostgreSQL inventory."""

from __future__ import annotations

import argparse
import json
import sqlite3
from collections import defaultdict
from contextlib import closing
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import psycopg

from ait import local_content, snapshot_diff
from ait.repo_paths import RepoContext, configured_repo_name


@dataclass(frozen=True)
class TaskRow:
    task_id: str
    repo_name: str
    title: str
    intent: str
    risk_tier: str
    status: str
    publication_state: str
    published_task_id: str | None
    plan_id: str | None
    origin_plan_revision_id: str | None
    plan_item_ref: str | None
    created_at: str
    updated_at: str


def _now_utc() -> datetime:
    return datetime.now(timezone.utc)


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _normalize_optional_text(value: Any) -> str | None:
    text = str(value or "").strip()
    return text or None


def _load_tasks(control_db_path: Path) -> list[TaskRow]:
    conn = sqlite3.connect(control_db_path)
    conn.row_factory = sqlite3.Row
    rows = conn.execute(
        """
        select
            task_id,
            repo_name,
            title,
            intent,
            risk_tier,
            status,
            publication_state,
            published_task_id,
            plan_id,
            origin_plan_revision_id,
            plan_item_ref,
            created_at,
            updated_at
        from workflow_tasks
        order by created_at asc, task_id asc
        """
    ).fetchall()
    conn.close()
    return [
        TaskRow(
            task_id=str(row["task_id"]),
            repo_name=str(row["repo_name"]),
            title=str(row["title"]),
            intent=str(row["intent"]),
            risk_tier=str(row["risk_tier"]),
            status=str(row["status"]),
            publication_state=str(row["publication_state"]),
            published_task_id=_normalize_optional_text(row["published_task_id"]),
            plan_id=_normalize_optional_text(row["plan_id"]),
            origin_plan_revision_id=_normalize_optional_text(row["origin_plan_revision_id"]),
            plan_item_ref=_normalize_optional_text(row["plan_item_ref"]),
            created_at=str(row["created_at"]),
            updated_at=str(row["updated_at"]),
        )
        for row in rows
    ]


def _load_changes_by_task(control_db_path: Path) -> dict[str, list[dict[str, Any]]]:
    conn = sqlite3.connect(control_db_path)
    conn.row_factory = sqlite3.Row
    rows = conn.execute(
        """
        select
            change_id,
            task_id,
            repo_name,
            title,
            status,
            publication_state,
            published_change_id,
            target_line,
            fork_snapshot_id,
            landed_snapshot_id,
            created_at,
            updated_at
        from workflow_changes
        order by created_at asc, change_id asc
        """
    ).fetchall()
    conn.close()
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        grouped[str(row["task_id"])].append(
            {
                "change_id": str(row["change_id"]),
                "task_id": str(row["task_id"]),
                "repo_name": str(row["repo_name"]),
                "title": str(row["title"]),
                "status": str(row["status"]),
                "publication_state": str(row["publication_state"]),
                "published_change_id": _normalize_optional_text(row["published_change_id"]),
                "target_line": _normalize_optional_text(row["target_line"]),
                "fork_snapshot_id": _normalize_optional_text(row["fork_snapshot_id"]),
                "landed_snapshot_id": _normalize_optional_text(row["landed_snapshot_id"]),
                "created_at": str(row["created_at"]),
                "updated_at": str(row["updated_at"]),
            }
        )
    return grouped


def _snapshot_file_map(
    content_conn: sqlite3.Connection,
    snapshot_cache: dict[str, dict[str, dict[str, Any]]],
    snapshot_id: str | None,
) -> dict[str, dict[str, Any]]:
    if not snapshot_id:
        return {}
    cached = snapshot_cache.get(snapshot_id)
    if cached is not None:
        return cached
    value = local_content._snapshot_file_map(content_conn, snapshot_id)
    snapshot_cache[snapshot_id] = value
    return value


def _changed_test_files(
    content_conn: sqlite3.Connection,
    snapshot_cache: dict[str, dict[str, dict[str, Any]]],
    *,
    base_snapshot_id: str | None,
    target_snapshot_id: str | None,
) -> list[str]:
    if not base_snapshot_id or not target_snapshot_id:
        return []
    base_files = _snapshot_file_map(content_conn, snapshot_cache, base_snapshot_id)
    target_files = _snapshot_file_map(content_conn, snapshot_cache, target_snapshot_id)
    diff = snapshot_diff.diff_snapshot_file_maps(
        base_files,
        target_files,
        old_snapshot_id=base_snapshot_id,
        new_snapshot_id=target_snapshot_id,
    )
    paths = (
        list(diff.get("added") or [])
        + list(diff.get("deleted") or [])
        + list(diff.get("modified") or [])
        + list(diff.get("mode_changed") or [])
    )
    return sorted({path for path in paths if str(path).startswith("tests/") and str(path).endswith(".py")})


def _load_inventory_by_file(
    *,
    dsn: str,
    control_schema: str,
    repo_id: str,
) -> dict[str, list[dict[str, Any]]]:
    with closing(psycopg.connect(dsn)) as conn:
        with conn.cursor(row_factory=psycopg.rows.dict_row) as cur:
            cur.execute(f'set search_path to "{control_schema}", public')
            cur.execute(
                """
                select
                    repo_id,
                    test_case_id,
                    pytest_node_id,
                    test_file_path,
                    class_name,
                    function_name,
                    description,
                    source_line
                from test_case_inventory
                where repo_id = %s
                order by test_file_path asc, source_line asc, function_name asc
                """,
                (repo_id,),
            )
            rows = list(cur.fetchall())
    grouped: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        grouped[str(row["test_file_path"])].append(
            {
                "repo_id": str(row["repo_id"]),
                "test_case_id": str(row["test_case_id"]),
                "pytest_node_id": str(row["pytest_node_id"]),
                "test_file_path": str(row["test_file_path"]),
                "class_name": _normalize_optional_text(row["class_name"]),
                "function_name": str(row["function_name"]),
                "description": str(row["description"]),
                "source_line": int(row["source_line"]),
            }
        )
    return grouped


def _task_payload(
    task: TaskRow,
    *,
    changes: list[dict[str, Any]],
    inventory_by_file: dict[str, list[dict[str, Any]]],
    content_conn: sqlite3.Connection,
    snapshot_cache: dict[str, dict[str, dict[str, Any]]],
) -> dict[str, Any]:
    task_test_files: dict[str, dict[str, Any]] = {}
    unresolved_changes: list[dict[str, Any]] = []
    snapshot_backed_changes = 0

    for change in changes:
        base_snapshot_id = change.get("fork_snapshot_id")
        target_snapshot_id = change.get("landed_snapshot_id")
        if not base_snapshot_id or not target_snapshot_id:
            unresolved_changes.append(
                {
                    "change_id": change["change_id"],
                    "status": change["status"],
                    "reason": "snapshot_diff_unavailable",
                    "fork_snapshot_id": base_snapshot_id,
                    "target_snapshot_id": target_snapshot_id,
                }
            )
            continue
        changed_test_files = _changed_test_files(
            content_conn,
            snapshot_cache,
            base_snapshot_id=base_snapshot_id,
            target_snapshot_id=target_snapshot_id,
        )
        snapshot_backed_changes += 1
        for path in changed_test_files:
            entry = task_test_files.setdefault(
                path,
                {
                    "test_file_path": path,
                    "source_change_ids": [],
                    "test_cases": [],
                },
            )
            if change["change_id"] not in entry["source_change_ids"]:
                entry["source_change_ids"].append(change["change_id"])
            known_case_ids = {row["test_case_id"] for row in entry["test_cases"]}
            for case in inventory_by_file.get(path, []):
                if case["test_case_id"] in known_case_ids:
                    continue
                entry["test_cases"].append(dict(case))
                known_case_ids.add(case["test_case_id"])

    test_file_rows = sorted(task_test_files.values(), key=lambda row: row["test_file_path"])
    test_cases: list[dict[str, Any]] = []
    seen_test_case_ids: set[str] = set()
    for file_row in test_file_rows:
        file_row["source_change_ids"].sort()
        file_row["test_cases"].sort(
            key=lambda row: (
                int(row["source_line"]),
                str(row["class_name"] or ""),
                str(row["function_name"]),
            )
        )
        for case in file_row["test_cases"]:
            if case["test_case_id"] in seen_test_case_ids:
                continue
            seen_test_case_ids.add(case["test_case_id"])
            test_cases.append(dict(case))

    expected_test_case_count = sum(len(file_row["test_cases"]) for file_row in test_file_rows)
    materialized_test_case_count = len(test_cases)

    reviewer_test_backfill = {
        "source": "snapshot_diff_to_test_case_inventory",
        "snapshot_backed_change_count": snapshot_backed_changes,
        "unresolved_change_count": len(unresolved_changes),
        "test_file_count": len(test_file_rows),
        "test_case_count": expected_test_case_count,
        "materialized_test_case_count": materialized_test_case_count,
        "counts_match": expected_test_case_count == materialized_test_case_count,
    }

    return {
        "task": {
            "task_id": task.task_id,
            "repo_name": task.repo_name,
            "title": task.title,
            "intent": task.intent,
            "risk_tier": task.risk_tier,
            "status": task.status,
            "publication_state": task.publication_state,
            "published_task_id": task.published_task_id,
            "plan_id": task.plan_id,
            "origin_plan_revision_id": task.origin_plan_revision_id,
            "plan_item_ref": task.plan_item_ref,
            "created_at": task.created_at,
            "updated_at": task.updated_at,
        },
        "changes": changes,
        "reviewer_test_backfill": reviewer_test_backfill,
        "test_files": test_file_rows,
        "test_cases": test_cases,
        "unresolved_changes": unresolved_changes,
    }


def export_task_test_case_backfill(
    *,
    repo_root: Path,
    dsn: str,
    control_schema: str,
    repo_id: str,
) -> dict[str, Any]:
    ctx = RepoContext.discover(repo_root)
    repo_name = configured_repo_name(ctx) or ctx.root.name
    tasks = _load_tasks(ctx.control_db_path)
    changes_by_task = _load_changes_by_task(ctx.control_db_path)
    inventory_by_file = _load_inventory_by_file(dsn=dsn, control_schema=control_schema, repo_id=repo_id)

    content_conn = sqlite3.connect(ctx.content_db_path)
    content_conn.row_factory = sqlite3.Row
    snapshot_cache: dict[str, dict[str, dict[str, Any]]] = {}
    try:
        task_rows = [
            _task_payload(
                task,
                changes=changes_by_task.get(task.task_id, []),
                inventory_by_file=inventory_by_file,
                content_conn=content_conn,
                snapshot_cache=snapshot_cache,
            )
            for task in tasks
            if task.repo_name == repo_name
        ]
    finally:
        content_conn.close()

    mismatch_count = sum(
        1
        for row in task_rows
        if not bool((row.get("reviewer_test_backfill") or {}).get("counts_match"))
    )
    total_test_cases = sum(int((row.get("reviewer_test_backfill") or {}).get("test_case_count") or 0) for row in task_rows)
    total_test_files = sum(int((row.get("reviewer_test_backfill") or {}).get("test_file_count") or 0) for row in task_rows)
    total_unresolved_changes = sum(
        int((row.get("reviewer_test_backfill") or {}).get("unresolved_change_count") or 0) for row in task_rows
    )
    return {
        "generated_at": _now_utc().isoformat(),
        "repo_name": repo_name,
        "repo_id": repo_id,
        "source": {
            "workflow_control_db": str(ctx.control_db_path),
            "content_db": str(ctx.content_db_path),
            "inventory_table": f"{control_schema}.test_case_inventory",
        },
        "summary": {
            "task_count": len(task_rows),
            "total_test_case_count": total_test_cases,
            "total_test_file_count": total_test_files,
            "total_unresolved_change_count": total_unresolved_changes,
            "reviewer_test_backfill_mismatch_count": mismatch_count,
        },
        "tasks": task_rows,
    }


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--repo-root",
        default=".",
        help="Repository root containing .ait control/content DBs. Defaults to current directory.",
    )
    parser.add_argument(
        "--dsn",
        required=True,
        help="PostgreSQL DSN holding test_case_inventory.",
    )
    parser.add_argument(
        "--control-schema",
        default="ait_native_control",
        help="PostgreSQL control schema containing test_case_inventory.",
    )
    parser.add_argument(
        "--repo-id",
        required=True,
        help="Repository id used in test_case_inventory lookups.",
    )
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    parser.add_argument("--json", action="store_true", help="Emit JSON. Kept for symmetry with other repo scripts.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    payload = export_task_test_case_backfill(
        repo_root=Path(args.repo_root).expanduser().resolve(),
        dsn=str(args.dsn),
        control_schema=str(args.control_schema),
        repo_id=str(args.repo_id),
    )
    text = _json_dump(payload)
    if args.output:
        output_path = Path(args.output).expanduser()
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(text + ("\n" if not text.endswith("\n") else ""), encoding="utf-8")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
