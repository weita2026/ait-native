#!/usr/bin/env python3
"""Resolve unmatched task-test-case links when a unique suitable task can be proven."""

from __future__ import annotations

import argparse
import ast
import json
import os
import sqlite3
from collections import defaultdict
from contextlib import closing
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from ait import local_content
from ait.repo_paths import RepoContext


def _now_utc() -> datetime:
    return datetime.now(timezone.utc)


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _normalize_schema_name(value: str, *, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    if not text.replace("_", "a").isalnum() or not (text[0].isalpha() or text[0] == "_"):
        raise ValueError(f"{field} must be a valid PostgreSQL schema identifier")
    return text


def _load_psycopg():
    import psycopg  # type: ignore

    return psycopg


@dataclass(frozen=True)
class CandidatePatchset:
    task_id: str
    change_id: str
    patchset_id: str
    base_snapshot_id: str | None
    revision_snapshot_id: str | None


def _function_segments(text: str | None) -> dict[str, str]:
    if not text:
        return {}
    tree = ast.parse(text)
    lines = text.splitlines()
    out: dict[str, str] = {}
    for node in ast.walk(tree):
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
            start = getattr(node, "lineno", None)
            end = getattr(node, "end_lineno", None)
            if start and end:
                out[node.name] = "\n".join(lines[start - 1 : end])
    return out


def _snapshot_file_text(
    ctx: RepoContext,
    conn: sqlite3.Connection,
    cache: dict[tuple[str, str], str | None],
    snapshot_id: str | None,
    test_file_path: str,
) -> str | None:
    key = (str(snapshot_id or ""), test_file_path)
    if key in cache:
        return cache[key]
    if not snapshot_id:
        cache[key] = None
        return None
    row = conn.execute(
        "select blob_id from snapshot_files where snapshot_id = ? and path = ?",
        (snapshot_id, test_file_path),
    ).fetchone()
    if row is None:
        cache[key] = None
        return None
    data = local_content._blob_bytes_by_id(ctx, conn, row["blob_id"])
    text = data.decode("utf-8", errors="replace")
    cache[key] = text
    return text


def _task_ids_that_change_function(
    *,
    ctx: RepoContext,
    content_conn: sqlite3.Connection,
    text_cache: dict[tuple[str, str], str | None],
    candidates: list[CandidatePatchset],
    test_file_path: str,
    function_name: str,
) -> tuple[set[str], dict[str, list[str]]]:
    task_ids: set[str] = set()
    patchsets_by_task: dict[str, list[str]] = defaultdict(list)
    for candidate in candidates:
        base_text = _snapshot_file_text(ctx, content_conn, text_cache, candidate.base_snapshot_id, test_file_path)
        rev_text = _snapshot_file_text(ctx, content_conn, text_cache, candidate.revision_snapshot_id, test_file_path)
        base_segments = _function_segments(base_text)
        rev_segments = _function_segments(rev_text)
        before = base_segments.get(function_name)
        after = rev_segments.get(function_name)
        if before is None and after is None:
            continue
        if before is None and after is not None:
            task_ids.add(candidate.task_id)
            patchsets_by_task[candidate.task_id].append(candidate.patchset_id)
            continue
        if before is not None and after is not None and before != after:
            task_ids.add(candidate.task_id)
            patchsets_by_task[candidate.task_id].append(candidate.patchset_id)
    return task_ids, {task_id: sorted(set(values)) for task_id, values in patchsets_by_task.items()}


def _change_ids_for_task_patchsets(
    candidates: list[CandidatePatchset],
    *,
    task_id: str,
    patchset_ids: list[str],
) -> list[str]:
    patchset_id_set = set(patchset_ids)
    return sorted(
        {
            candidate.change_id
            for candidate in candidates
            if candidate.task_id == task_id and candidate.patchset_id in patchset_id_set
        }
    )


def _choose_task_for_test_case(
    *,
    ctx: RepoContext,
    content_conn: sqlite3.Connection,
    text_cache: dict[tuple[str, str], str | None],
    candidate_patchsets: list[CandidatePatchset],
    candidate_task_ids: list[str],
    test_file_path: str,
    function_name: str,
    function_count: int,
) -> tuple[str | None, list[str], list[str], str | None]:
    if not candidate_task_ids:
        return None, [], [], None

    changed_task_ids, patchsets_by_task = _task_ids_that_change_function(
        ctx=ctx,
        content_conn=content_conn,
        text_cache=text_cache,
        candidates=candidate_patchsets,
        test_file_path=test_file_path,
        function_name=function_name,
    )

    if len(candidate_task_ids) == 1:
        chosen_task_id = candidate_task_ids[0]
        changed_patchset_ids = patchsets_by_task.get(chosen_task_id, [])
        if changed_task_ids == {chosen_task_id} and changed_patchset_ids:
            return (
                chosen_task_id,
                changed_patchset_ids,
                _change_ids_for_task_patchsets(candidate_patchsets, task_id=chosen_task_id, patchset_ids=changed_patchset_ids),
                "single_candidate_task_and_function_diff",
            )
        if function_count == 1:
            chosen_patchset_ids = sorted({candidate.patchset_id for candidate in candidate_patchsets})
            return (
                chosen_task_id,
                chosen_patchset_ids,
                _change_ids_for_task_patchsets(candidate_patchsets, task_id=chosen_task_id, patchset_ids=chosen_patchset_ids),
                "single_candidate_task_and_unique_function_name",
            )
        return None, [], [], None

    if function_count == 1 and len(changed_task_ids) == 1:
        chosen_task_id = sorted(changed_task_ids)[0]
        chosen_patchset_ids = patchsets_by_task.get(chosen_task_id, [])
        return (
            chosen_task_id,
            chosen_patchset_ids,
            _change_ids_for_task_patchsets(candidate_patchsets, task_id=chosen_task_id, patchset_ids=chosen_patchset_ids),
            "unique_function_diff_task_among_multi_candidates",
        )

    return None, [], [], None


def resolve_unmatched_task_test_cases(
    *,
    dsn: str,
    repo_root: Path,
    control_schema: str,
    repo_id: str,
) -> dict[str, Any]:
    psycopg = _load_psycopg()
    normalized_control_schema = _normalize_schema_name(control_schema, field="control_schema")
    ctx = RepoContext.discover(repo_root)
    content_conn = sqlite3.connect(ctx.content_db_path)
    content_conn.row_factory = sqlite3.Row
    text_cache: dict[tuple[str, str], str | None] = {}

    with closing(psycopg.connect(dsn, row_factory=psycopg.rows.dict_row)) as connection:
        with connection.cursor() as cursor:
            cursor.execute(f'set search_path to "{normalized_control_schema}", public')
            cursor.execute(
                """
                with unmatched as (
                    select
                        t.repo_name,
                        t.repo_id,
                        t.test_case_id,
                        t.pytest_node_id,
                        t.test_file_path,
                        t.class_name,
                        t.function_name,
                        t.source_line
                    from test_case_inventory t
                    where t.repo_id = %s
                      and not exists (
                          select 1
                          from task_test_case_links l
                          where l.repo_id = t.repo_id
                            and l.test_case_id = t.test_case_id
                      )
                )
                select * from unmatched order by test_file_path asc, source_line asc, function_name asc
                """,
                (repo_id,),
            )
            unmatched = list(cursor.fetchall())

            cursor.execute(
                """
                select function_name, count(*) as c
                from test_case_inventory
                where repo_id = %s
                group by function_name
                """,
                (repo_id,),
            )
            function_counts = {str(row["function_name"]): int(row["c"]) for row in cursor.fetchall()}

            cursor.execute(
                """
                with patch_paths as (
                    select
                        p.patchset_id,
                        p.change_id,
                        c.task_id,
                        p.base_snapshot_id,
                        p.revision_snapshot_id,
                        jsonb_array_elements_text(coalesce(p.diff_stats_json->'paths'->'modified', '[]'::jsonb)) as path
                    from remote_patchset_inventory p
                    join remote_change_inventory c on c.repo_id = p.repo_id and c.change_id = p.change_id
                    where p.repo_id = %s
                    union all
                    select
                        p.patchset_id,
                        p.change_id,
                        c.task_id,
                        p.base_snapshot_id,
                        p.revision_snapshot_id,
                        jsonb_array_elements_text(coalesce(p.diff_stats_json->'paths'->'added', '[]'::jsonb)) as path
                    from remote_patchset_inventory p
                    join remote_change_inventory c on c.repo_id = p.repo_id and c.change_id = p.change_id
                    where p.repo_id = %s
                    union all
                    select
                        p.patchset_id,
                        p.change_id,
                        c.task_id,
                        p.base_snapshot_id,
                        p.revision_snapshot_id,
                        jsonb_array_elements_text(coalesce(p.diff_stats_json->'paths'->'deleted', '[]'::jsonb)) as path
                    from remote_patchset_inventory p
                    join remote_change_inventory c on c.repo_id = p.repo_id and c.change_id = p.change_id
                    where p.repo_id = %s
                )
                select *
                from patch_paths
                order by path asc, task_id asc, patchset_id asc
                """,
                (repo_id, repo_id, repo_id),
            )
            candidate_rows = list(cursor.fetchall())

            candidates_by_path: dict[str, list[CandidatePatchset]] = defaultdict(list)
            for row in candidate_rows:
                candidates_by_path[str(row["path"])].append(
                    CandidatePatchset(
                        task_id=str(row["task_id"]),
                        change_id=str(row["change_id"]),
                        patchset_id=str(row["patchset_id"]),
                        base_snapshot_id=str(row["base_snapshot_id"]) if row["base_snapshot_id"] else None,
                        revision_snapshot_id=str(row["revision_snapshot_id"]) if row["revision_snapshot_id"] else None,
                    )
                )

            cursor.execute(
                "select task_id, title from remote_task_inventory where repo_id = %s",
                (repo_id,),
            )
            task_titles = {str(row["task_id"]): str(row["title"]) for row in cursor.fetchall()}

            resolved_links: list[dict[str, Any]] = []
            unresolved: list[dict[str, Any]] = []
            linked_at = _now_utc().isoformat()

            for row in unmatched:
                test_case = dict(row)
                function_name = str(test_case["function_name"])
                test_file_path = str(test_case["test_file_path"])
                candidate_patchsets = candidates_by_path.get(test_file_path, [])
                candidate_task_ids = sorted({candidate.task_id for candidate in candidate_patchsets})
                function_count = int(function_counts.get(function_name, 0))

                chosen_task_id, chosen_patchset_ids, chosen_change_ids, resolution_reason = _choose_task_for_test_case(
                    ctx=ctx,
                    content_conn=content_conn,
                    text_cache=text_cache,
                    candidate_patchsets=candidate_patchsets,
                    candidate_task_ids=candidate_task_ids,
                    test_file_path=test_file_path,
                    function_name=function_name,
                    function_count=function_count,
                )

                if chosen_task_id is None:
                    unresolved.append(
                        {
                            "repo_name": str(test_case["repo_name"]),
                            "repo_id": str(test_case["repo_id"]),
                            "test_case_id": str(test_case["test_case_id"]),
                            "pytest_node_id": str(test_case["pytest_node_id"]),
                            "test_file_path": test_file_path,
                            "function_name": function_name,
                            "candidate_task_ids": candidate_task_ids,
                            "candidate_task_count": len(candidate_task_ids),
                            "function_name_inventory_count": function_count,
                            "reason": "no_unique_suitable_task",
                        }
                    )
                    continue

                resolved_links.append(
                    {
                        "repo_name": str(test_case["repo_name"]),
                        "repo_id": str(test_case["repo_id"]),
                        "task_id": chosen_task_id,
                        "task_title": task_titles.get(chosen_task_id),
                        "test_case_id": str(test_case["test_case_id"]),
                        "pytest_node_id": str(test_case["pytest_node_id"]),
                        "test_file_path": test_file_path,
                        "class_name": test_case.get("class_name"),
                        "function_name": function_name,
                        "source_line": int(test_case["source_line"]),
                        "source_change_ids": chosen_change_ids,
                        "source_patchset_ids": chosen_patchset_ids,
                        "verification_mode": resolution_reason,
                        "linked_at": linked_at,
                    }
                )

            for link in resolved_links:
                cursor.execute(
                    """
                    insert into task_test_case_links (
                        repo_name, repo_id, task_id, test_case_id, pytest_node_id, test_file_path,
                        class_name, function_name, source_line, source_change_ids_json,
                        source_patchset_ids_json, verification_mode, linked_at
                    )
                    values (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s::jsonb, %s::jsonb, %s, %s)
                    on conflict (repo_id, task_id, test_case_id) do update set
                        pytest_node_id = excluded.pytest_node_id,
                        test_file_path = excluded.test_file_path,
                        class_name = excluded.class_name,
                        function_name = excluded.function_name,
                        source_line = excluded.source_line,
                        source_change_ids_json = excluded.source_change_ids_json,
                        source_patchset_ids_json = excluded.source_patchset_ids_json,
                        verification_mode = excluded.verification_mode,
                        linked_at = excluded.linked_at
                    """,
                    (
                        link["repo_name"],
                        link["repo_id"],
                        link["task_id"],
                        link["test_case_id"],
                        link["pytest_node_id"],
                        link["test_file_path"],
                        link["class_name"],
                        link["function_name"],
                        link["source_line"],
                        json.dumps(link["source_change_ids"], ensure_ascii=False),
                        json.dumps(link["source_patchset_ids"], ensure_ascii=False),
                        link["verification_mode"],
                        link["linked_at"],
                    ),
                )
        connection.commit()
    content_conn.close()

    summary = {
        "starting_unmatched_count": len(unmatched),
        "resolved_link_count": len(resolved_links),
        "remaining_unresolved_count": len(unresolved),
    }
    return {
        "generated_at": _now_utc().isoformat(),
        "repo_id": repo_id,
        "summary": summary,
        "resolved_links": resolved_links,
        "unresolved": unresolved,
    }


def _build_unresolved_markdown(payload: dict[str, Any]) -> str:
    lines = [
        f"# Remaining Unresolved Task Test Case Links ({payload['repo_id']})",
        "",
        f"- generated_at: {payload['generated_at']}",
        f"- starting_unmatched_count: {payload['summary']['starting_unmatched_count']}",
        f"- resolved_link_count: {payload['summary']['resolved_link_count']}",
        f"- remaining_unresolved_count: {payload['summary']['remaining_unresolved_count']}",
        "",
    ]
    for row in payload["unresolved"]:
        lines.append(f"## {row['test_case_id']} {row['function_name']}")
        lines.append(f"- file: {row['test_file_path']}")
        lines.append(f"- candidate_task_count: {row['candidate_task_count']}")
        lines.append(f"- function_name_inventory_count: {row['function_name_inventory_count']}")
        lines.append(f"- candidate_task_ids: {', '.join(row['candidate_task_ids']) if row['candidate_task_ids'] else 'none'}")
        lines.append("")
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dsn", default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN"), help="PostgreSQL DSN.")
    parser.add_argument("--repo-root", default=".", help="Repository root.")
    parser.add_argument("--control-schema", default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control"))
    parser.add_argument("--repo-id", required=True, help="Remote repo_id to process.")
    parser.add_argument("--output", help="Optional JSON output path.")
    parser.add_argument("--unresolved-markdown", help="Optional unresolved Markdown output path.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    dsn = str(args.dsn or "").strip()
    if not dsn:
        parser.error("Pass --dsn or set AIT_NATIVE_SERVER_POSTGRES_DSN.")
    payload = resolve_unmatched_task_test_cases(
        dsn=dsn,
        repo_root=Path(args.repo_root).expanduser().resolve(),
        control_schema=str(args.control_schema),
        repo_id=str(args.repo_id),
    )
    text = _json_dump(payload)
    if args.output:
        path = Path(args.output).expanduser()
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text + ("" if text.endswith("\n") else "\n"), encoding="utf-8")
    else:
        print(text)
    if args.unresolved_markdown:
        md_path = Path(args.unresolved_markdown).expanduser()
        md_path.parent.mkdir(parents=True, exist_ok=True)
        md_text = _build_unresolved_markdown(payload)
        md_path.write_text(md_text + ("" if md_text.endswith("\n") else "\n"), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
