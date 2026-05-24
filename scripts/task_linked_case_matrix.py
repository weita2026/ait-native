#!/usr/bin/env python3
"""Run per-task pytest coverage from PostgreSQL task-test-case links."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import time
from contextlib import closing
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def _now_utc() -> str:
    return datetime.now(timezone.utc).isoformat()


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _truncate_text(value: str, *, limit: int = 4000) -> str:
    text = str(value or "")
    if len(text) <= limit:
        return text
    return text[: limit - 15] + "\n...<truncated>"


def _normalize_schema_name(value: str, *, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    if not text.replace("_", "a").isalnum() or not (text[0].isalpha() or text[0] == "_"):
        raise ValueError(f"{field} must be a valid PostgreSQL schema identifier")
    return text


def _optional_text(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _load_psycopg():
    import psycopg  # type: ignore

    return psycopg


def _discover_repo_name(repo_root: Path) -> str:
    config_path = repo_root / ".ait" / "config.json"
    payload = json.loads(config_path.read_text(encoding="utf-8"))
    repo_name = str(payload.get("repo_name") or "").strip()
    if not repo_name:
        raise RuntimeError(f"repo_name is missing from {config_path}")
    return repo_name


def _resolve_repo_id(cursor: Any, *, repo_id: str | None, repo_name: str) -> str:
    normalized_repo_id = str(repo_id or "").strip()
    if normalized_repo_id:
        return normalized_repo_id
    cursor.execute(
        """
        select repo_id
        from task_test_case_links
        where repo_name = %s
        group by repo_id
        order by count(*) desc, repo_id asc
        limit 2
        """,
        (repo_name,),
    )
    rows = list(cursor.fetchall())
    if not rows:
        raise RuntimeError(f"No task_test_case_links rows found for repo `{repo_name}`.")
    if len(rows) > 1:
        raise RuntimeError(
            f"Multiple repo_id values found for repo `{repo_name}` in task_test_case_links; pass --repo-id explicitly."
        )
    return str(rows[0]["repo_id"])


def _load_task_matrix_rows(cursor: Any, *, repo_id: str, task_ids: list[str] | None) -> list[dict[str, Any]]:
    params: list[Any] = [repo_id]
    task_filter = ""
    normalized_task_ids = [str(item).strip() for item in list(task_ids or []) if str(item).strip()]
    if normalized_task_ids:
        task_filter = "and l.task_id = any(%s)"
        params.append(normalized_task_ids)
    cursor.execute(
        f"""
        select
            l.task_id,
            coalesce(t.title, '') as task_title,
            array_agg(distinct l.pytest_node_id order by l.pytest_node_id) as pytest_node_ids
        from task_test_case_links l
        left join remote_task_inventory t on t.repo_id = l.repo_id and t.task_id = l.task_id
        where l.repo_id = %s
          {task_filter}
        group by l.task_id, t.title
        order by l.task_id asc
        """,
        tuple(params),
    )
    rows = []
    for row in cursor.fetchall():
        node_ids = [str(item).strip() for item in list(row["pytest_node_ids"] or []) if str(item).strip()]
        rows.append(
            {
                "task_id": str(row["task_id"]),
                "task_title": str(row.get("task_title") or ""),
                "pytest_node_ids": node_ids,
            }
        )
    return rows


def _build_pytest_env() -> dict[str, str]:
    env = os.environ.copy()
    existing = str(env.get("PYTHONPATH") or "").strip()
    env["PYTHONPATH"] = f"src{os.pathsep}{existing}" if existing else "src"
    return env


def run_task_linked_case_matrix(
    *,
    dsn: str,
    repo_root: Path,
    control_schema: str,
    repo_id: str | None,
    repo_name: str | None,
    task_ids: list[str] | None,
    max_tasks: int | None,
    fail_fast: bool,
) -> dict[str, Any]:
    psycopg = _load_psycopg()
    normalized_control_schema = _normalize_schema_name(control_schema, field="control_schema")
    resolved_repo_name = str(repo_name or "").strip() or _discover_repo_name(repo_root)

    with closing(psycopg.connect(dsn, row_factory=psycopg.rows.dict_row)) as connection:
        with connection.cursor() as cursor:
            cursor.execute(f'set search_path to "{normalized_control_schema}", public')
            resolved_repo_id = _resolve_repo_id(cursor, repo_id=repo_id, repo_name=resolved_repo_name)
            task_rows = _load_task_matrix_rows(cursor, repo_id=resolved_repo_id, task_ids=task_ids)

    if max_tasks is not None and max_tasks >= 0:
        task_rows = task_rows[:max_tasks]

    env = _build_pytest_env()
    task_results: list[dict[str, Any]] = []
    total_test_nodes = 0
    started_at = _now_utc()

    for row in task_rows:
        task_id = str(row["task_id"])
        node_ids = [str(item) for item in list(row["pytest_node_ids"] or []) if str(item).strip()]
        total_test_nodes += len(node_ids)
        command = [sys.executable, "-m", "pytest", "-q", *node_ids]
        started = time.monotonic()
        completed = subprocess.run(
            command,
            cwd=repo_root,
            env=env,
            text=True,
            capture_output=True,
        )
        duration = round(time.monotonic() - started, 3)
        task_result = {
            "task_id": task_id,
            "task_title": str(row.get("task_title") or ""),
            "pytest_node_count": len(node_ids),
            "pytest_node_ids": node_ids,
            "command": " ".join(command),
            "status": "pass" if completed.returncode == 0 else "fail",
            "exit_code": int(completed.returncode),
            "duration_seconds": duration,
            "stdout": _truncate_text(completed.stdout),
            "stderr": _truncate_text(completed.stderr),
        }
        task_results.append(task_result)
        if completed.returncode != 0 and fail_fast:
            break

    failed_task_ids = [str(row["task_id"]) for row in task_results if row["status"] != "pass"]
    payload = {
        "generated_at": _now_utc(),
        "started_at": started_at,
        "repo_name": resolved_repo_name,
        "repo_id": resolved_repo_id,
        "control_schema": normalized_control_schema,
        "selected_task_count": len(task_rows),
        "executed_task_count": len(task_results),
        "failed_task_count": len(failed_task_ids),
        "passed_task_count": len(task_results) - len(failed_task_ids),
        "total_pytest_node_count": total_test_nodes,
        "status": "pass" if not failed_task_ids else "fail",
        "fail_fast": bool(fail_fast),
        "task_results": task_results,
        "failed_task_ids": failed_task_ids,
    }
    return payload


def _build_markdown(payload: dict[str, Any]) -> str:
    lines = [
        f"# Task Linked Case Matrix ({payload['repo_name']})",
        "",
        f"- repo_id: `{payload['repo_id']}`",
        f"- selected_task_count: {payload['selected_task_count']}",
        f"- executed_task_count: {payload['executed_task_count']}",
        f"- passed_task_count: {payload['passed_task_count']}",
        f"- failed_task_count: {payload['failed_task_count']}",
        f"- total_pytest_node_count: {payload['total_pytest_node_count']}",
        f"- status: `{payload['status']}`",
        "",
        "## Task Results",
        "",
    ]
    if not payload["task_results"]:
        lines.append("- none")
        return "\n".join(lines) + "\n"
    for row in payload["task_results"]:
        title = str(row.get("task_title") or "").strip()
        suffix = f" {title}" if title else ""
        lines.append(
            f"- `{row['task_id']}`{suffix}: `{row['status']}` "
            f"nodes={row['pytest_node_count']} exit={row['exit_code']} duration={row['duration_seconds']}s"
        )
    return "\n".join(lines).rstrip() + "\n"


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dsn", default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN"), help="PostgreSQL DSN.")
    parser.add_argument("--repo-root", default=".", help="Repository root.")
    parser.add_argument(
        "--control-schema",
        default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control"),
        help="PostgreSQL control schema.",
    )
    parser.add_argument("--repo-id", help="Explicit repo_id. Defaults to lookup by repo_name.")
    parser.add_argument("--repo-name", help="Explicit repo_name. Defaults to .ait/config.json repo_name.")
    parser.add_argument("--task-id", action="append", default=[], help="Optional task ids to limit the run to.")
    parser.add_argument("--max-tasks", type=int, help="Optional maximum number of tasks to execute.")
    parser.add_argument("--fail-fast", action="store_true", help="Stop after the first failing task.")
    parser.add_argument("--output", help="Optional JSON output path.")
    parser.add_argument("--markdown", help="Optional Markdown summary path.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    dsn = str(args.dsn or "").strip()
    if not dsn:
        parser.error("Pass --dsn or set AIT_NATIVE_SERVER_POSTGRES_DSN.")
    payload = run_task_linked_case_matrix(
        dsn=dsn,
        repo_root=Path(args.repo_root).expanduser().resolve(),
        control_schema=str(args.control_schema),
        repo_id=_optional_text(args.repo_id),
        repo_name=_optional_text(args.repo_name),
        task_ids=[str(item) for item in list(args.task_id or []) if str(item).strip()],
        max_tasks=args.max_tasks,
        fail_fast=bool(args.fail_fast),
    )
    text = _json_dump(payload)
    if args.output:
        output_path = Path(args.output).expanduser()
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(text + ("" if text.endswith("\n") else "\n"), encoding="utf-8")
    else:
        print(text)
    if args.markdown:
        markdown_path = Path(args.markdown).expanduser()
        markdown_path.parent.mkdir(parents=True, exist_ok=True)
        markdown = _build_markdown(payload)
        markdown_path.write_text(markdown, encoding="utf-8")
    return 0 if payload["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
