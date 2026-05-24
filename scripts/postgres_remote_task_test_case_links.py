#!/usr/bin/env python3
"""Sync remote task/change/patchset inventories and build task-test-case links."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import sys
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor, as_completed
from contextlib import closing
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable

from ait import remote_client
from ait.repo_paths import RepoContext, configured_repo_name
from ait.store import get_remote


TASK_TABLE = "remote_task_inventory"
CHANGE_TABLE = "remote_change_inventory"
PATCHSET_TABLE = "remote_patchset_inventory"
LINK_TABLE = "task_test_case_links"
LEGACY_JSON_PATH = Path(".ait/generated/reviewer_test_backfill/task_test_case_backfill_2026-05-13.json")


def _now_utc() -> datetime:
    return datetime.now(timezone.utc)


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _normalize_text(value: Any) -> str | None:
    text = str(value or "").strip()
    return text or None


def _normalize_schema_name(value: str, *, field: str) -> str:
    text = str(value or "").strip()
    if not text:
        raise ValueError(f"{field} is required")
    if not text.replace("_", "a").isalnum() or not (text[0].isalpha() or text[0] == "_"):
        raise ValueError(f"{field} must be a valid PostgreSQL schema identifier")
    return text


def _load_psycopg():
    try:
        import psycopg  # type: ignore
    except ModuleNotFoundError as exc:
        raise RuntimeError("This script requires psycopg. Install with: pip install 'ait-native[postgres]'") from exc
    return psycopg


def _load_inventory_script_module():
    module_name = "postgres_test_case_inventory_runtime"
    if module_name in sys.modules:
        return sys.modules[module_name]
    script_path = Path(__file__).with_name("postgres_test_case_inventory.py")
    spec = importlib.util.spec_from_file_location(module_name, script_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Could not load inventory sync module from {script_path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


def _resolve_repo_context(repo_root: Path) -> tuple[RepoContext, str]:
    ctx = RepoContext.discover(repo_root)
    repo_name = configured_repo_name(ctx) or ctx.root.name
    return ctx, repo_name


def _resolve_remote_url(ctx: RepoContext, remote_name: str) -> str:
    remote = get_remote(ctx, remote_name)
    url = _normalize_text(remote.get("url"))
    if not url:
        raise RuntimeError(f"Remote {remote_name!r} is missing a URL")
    return url


def _ensure_supporting_tables(cursor: Any, *, control_schema: str) -> None:
    cursor.execute(f'create schema if not exists "{control_schema}"')
    cursor.execute(f'set search_path to "{control_schema}", public')
    cursor.execute(
        f"""
        create table if not exists {TASK_TABLE} (
            repo_name text not null,
            repo_id text not null,
            task_id text not null,
            task_seq integer,
            title text not null,
            intent text not null,
            risk_tier text not null,
            status text not null,
            planning_state text,
            plan_id text,
            origin_plan_revision_id text,
            plan_item_ref text,
            created_at timestamptz not null,
            synced_at timestamptz not null,
            raw_json jsonb not null,
            primary key (repo_id, task_id)
        )
        """
    )
    cursor.execute(
        f"""
        create table if not exists {CHANGE_TABLE} (
            repo_name text not null,
            repo_id text not null,
            change_id text not null,
            change_seq integer,
            task_id text not null,
            title text not null,
            base_line text,
            lane text,
            risk_tier text,
            status text not null,
            fork_snapshot_id text,
            forked_from_line text,
            current_patchset_id text,
            current_patchset_number integer,
            selected_patchset_id text,
            selected_patchset_number integer,
            landed_at timestamptz,
            created_at timestamptz not null,
            updated_at timestamptz not null,
            synced_at timestamptz not null,
            raw_json jsonb not null,
            primary key (repo_id, change_id)
        )
        """
    )
    cursor.execute(
        f"""
        create table if not exists {PATCHSET_TABLE} (
            repo_name text not null,
            repo_id text not null,
            patchset_id text not null,
            change_id text not null,
            patchset_number integer,
            publish_state text,
            evaluation_state text,
            author_mode text,
            summary text,
            base_snapshot_id text,
            revision_snapshot_id text,
            diff_stats_json jsonb not null,
            created_at timestamptz not null,
            synced_at timestamptz not null,
            raw_json jsonb not null,
            primary key (repo_id, patchset_id)
        )
        """
    )
    cursor.execute(
        f"""
        create table if not exists {LINK_TABLE} (
            repo_name text not null,
            repo_id text not null,
            task_id text not null,
            test_case_id text not null,
            pytest_node_id text not null,
            test_file_path text not null,
            class_name text,
            function_name text not null,
            source_line integer not null,
            source_change_ids_json jsonb not null,
            source_patchset_ids_json jsonb not null,
            verification_mode text not null,
            linked_at timestamptz not null,
            primary key (repo_id, task_id, test_case_id)
        )
        """
    )
    cursor.execute(f"create index if not exists idx_{TASK_TABLE}_repo_task_seq on {TASK_TABLE}(repo_id, task_seq, task_id)")
    cursor.execute(f"create index if not exists idx_{CHANGE_TABLE}_repo_task on {CHANGE_TABLE}(repo_id, task_id, change_id)")
    cursor.execute(
        f"create index if not exists idx_{PATCHSET_TABLE}_repo_change on {PATCHSET_TABLE}(repo_id, change_id, patchset_number)"
    )
    cursor.execute(f"create index if not exists idx_{LINK_TABLE}_repo_task on {LINK_TABLE}(repo_id, task_id, test_case_id)")


def _upsert_remote_tasks(cursor: Any, *, control_schema: str, rows: list[dict[str, Any]], synced_at: str) -> None:
    cursor.execute(f'set search_path to "{control_schema}", public')
    for row in rows:
        cursor.execute(
            f"""
            insert into {TASK_TABLE} (
                repo_name, repo_id, task_id, task_seq, title, intent, risk_tier, status,
                planning_state, plan_id, origin_plan_revision_id, plan_item_ref, created_at, synced_at, raw_json
            )
            values (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s::jsonb)
            on conflict (repo_id, task_id) do update set
                task_seq = excluded.task_seq,
                title = excluded.title,
                intent = excluded.intent,
                risk_tier = excluded.risk_tier,
                status = excluded.status,
                planning_state = excluded.planning_state,
                plan_id = excluded.plan_id,
                origin_plan_revision_id = excluded.origin_plan_revision_id,
                plan_item_ref = excluded.plan_item_ref,
                created_at = excluded.created_at,
                synced_at = excluded.synced_at,
                raw_json = excluded.raw_json
            """,
            (
                row["repo_name"],
                row["repo_id"],
                row["task_id"],
                row.get("task_seq"),
                row["title"],
                row["intent"],
                row["risk_tier"],
                row["status"],
                row.get("planning_state"),
                row.get("plan_id"),
                row.get("origin_plan_revision_id"),
                row.get("plan_item_ref"),
                row["created_at"],
                synced_at,
                json.dumps(row, sort_keys=True, ensure_ascii=False),
            ),
        )


def _upsert_remote_changes(cursor: Any, *, control_schema: str, rows: list[dict[str, Any]], synced_at: str) -> None:
    cursor.execute(f'set search_path to "{control_schema}", public')
    for row in rows:
        cursor.execute(
            f"""
            insert into {CHANGE_TABLE} (
                repo_name, repo_id, change_id, change_seq, task_id, title, base_line, lane, risk_tier,
                status, fork_snapshot_id, forked_from_line, current_patchset_id, current_patchset_number,
                selected_patchset_id, selected_patchset_number, landed_at, created_at, updated_at, synced_at, raw_json
            )
            values (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s::jsonb)
            on conflict (repo_id, change_id) do update set
                change_seq = excluded.change_seq,
                task_id = excluded.task_id,
                title = excluded.title,
                base_line = excluded.base_line,
                lane = excluded.lane,
                risk_tier = excluded.risk_tier,
                status = excluded.status,
                fork_snapshot_id = excluded.fork_snapshot_id,
                forked_from_line = excluded.forked_from_line,
                current_patchset_id = excluded.current_patchset_id,
                current_patchset_number = excluded.current_patchset_number,
                selected_patchset_id = excluded.selected_patchset_id,
                selected_patchset_number = excluded.selected_patchset_number,
                landed_at = excluded.landed_at,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at,
                synced_at = excluded.synced_at,
                raw_json = excluded.raw_json
            """,
            (
                row["repo_name"],
                row["repo_id"],
                row["change_id"],
                row.get("change_seq"),
                row["task_id"],
                row["title"],
                row.get("base_line"),
                row.get("lane"),
                row.get("risk_tier"),
                row["status"],
                row.get("fork_snapshot_id"),
                row.get("forked_from_line"),
                row.get("current_patchset_id"),
                row.get("current_patchset_number"),
                row.get("selected_patchset_id"),
                row.get("selected_patchset_number"),
                row.get("landed_at"),
                row["created_at"],
                row["updated_at"],
                synced_at,
                json.dumps(row, sort_keys=True, ensure_ascii=False),
            ),
        )


def _upsert_remote_patchsets(
    cursor: Any,
    *,
    control_schema: str,
    rows: list[dict[str, Any]],
    repo_name: str,
    repo_id: str,
    synced_at: str,
) -> None:
    cursor.execute(f'set search_path to "{control_schema}", public')
    for row in rows:
        patchset_repo_name = _normalize_text(row.get("repo_name")) or repo_name
        patchset_repo_id = _normalize_text(row.get("repo_id")) or repo_id
        cursor.execute(
            f"""
            insert into {PATCHSET_TABLE} (
                repo_name, repo_id, patchset_id, change_id, patchset_number, publish_state,
                evaluation_state, author_mode, summary, base_snapshot_id, revision_snapshot_id,
                diff_stats_json, created_at, synced_at, raw_json
            )
            values (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s::jsonb, %s, %s, %s::jsonb)
            on conflict (repo_id, patchset_id) do update set
                change_id = excluded.change_id,
                patchset_number = excluded.patchset_number,
                publish_state = excluded.publish_state,
                evaluation_state = excluded.evaluation_state,
                author_mode = excluded.author_mode,
                summary = excluded.summary,
                base_snapshot_id = excluded.base_snapshot_id,
                revision_snapshot_id = excluded.revision_snapshot_id,
                diff_stats_json = excluded.diff_stats_json,
                created_at = excluded.created_at,
                synced_at = excluded.synced_at,
                raw_json = excluded.raw_json
            """,
            (
                patchset_repo_name,
                patchset_repo_id,
                row["patchset_id"],
                row["change_id"],
                row.get("patchset_number"),
                row.get("publish_state"),
                row.get("evaluation_state"),
                row.get("author_mode"),
                row.get("summary"),
                row.get("base_snapshot_id"),
                row.get("revision_snapshot_id"),
                json.dumps(row.get("diff_stats") or {}, sort_keys=True, ensure_ascii=False),
                row["created_at"],
                synced_at,
                json.dumps(row, sort_keys=True, ensure_ascii=False),
            ),
        )


def _load_inventory_rows(cursor: Any, *, control_schema: str, repo_id: str) -> tuple[dict[str, list[dict[str, Any]]], dict[str, list[dict[str, Any]]]]:
    cursor.execute(f'set search_path to "{control_schema}", public')
    cursor.execute(
        """
        select
            repo_name,
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
    rows = list(cursor.fetchall())
    by_path: dict[str, list[dict[str, Any]]] = defaultdict(list)
    by_function: dict[str, list[dict[str, Any]]] = defaultdict(list)
    for row in rows:
        normalized = {
            "repo_name": row["repo_name"],
            "repo_id": row["repo_id"],
            "test_case_id": row["test_case_id"],
            "pytest_node_id": row["pytest_node_id"],
            "test_file_path": row["test_file_path"],
            "class_name": row["class_name"],
            "function_name": row["function_name"],
            "description": row["description"],
            "source_line": int(row["source_line"]),
        }
        by_path[str(row["test_file_path"])].append(normalized)
        by_function[str(row["function_name"])].append(normalized)
    return by_path, by_function


def _coerce_json_value(value: Any) -> Any:
    if isinstance(value, (dict, list)):
        return value
    if value is None:
        return None
    return json.loads(str(value))


def _load_remote_db_rows(cursor: Any, *, control_schema: str, repo_id: str) -> tuple[list[dict[str, Any]], list[dict[str, Any]], dict[str, dict[str, Any]]]:
    cursor.execute(f'set search_path to "{control_schema}", public')
    cursor.execute(f"select raw_json from {TASK_TABLE} where repo_id = %s order by coalesce(task_seq, 0) asc, task_id asc", (repo_id,))
    task_rows = [_coerce_json_value(row["raw_json"]) for row in cursor.fetchall()]
    cursor.execute(f"select raw_json from {CHANGE_TABLE} where repo_id = %s order by coalesce(change_seq, 0) asc, change_id asc", (repo_id,))
    change_rows = [_coerce_json_value(row["raw_json"]) for row in cursor.fetchall()]
    cursor.execute(f"select patchset_id, raw_json from {PATCHSET_TABLE} where repo_id = %s order by patchset_id asc", (repo_id,))
    patchset_rows = {str(row["patchset_id"]): _coerce_json_value(row["raw_json"]) for row in cursor.fetchall()}
    return task_rows, change_rows, patchset_rows


def extract_changed_test_files(diff_stats: dict[str, Any] | None) -> list[str]:
    paths = ((diff_stats or {}).get("paths") or {}) if isinstance(diff_stats, dict) else {}
    combined = list(paths.get("added") or []) + list(paths.get("modified") or []) + list(paths.get("deleted") or [])
    return sorted({str(path) for path in combined if str(path).startswith("tests/") and str(path).endswith(".py")})


def _unique_case_rows(rows: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    by_id: dict[str, dict[str, Any]] = {}
    for row in rows:
        by_id[str(row["test_case_id"])] = dict(row)
    return sorted(
        by_id.values(),
        key=lambda row: (str(row["test_file_path"]), int(row["source_line"]), str(row["function_name"])),
    )


def build_task_verification(
    task: dict[str, Any],
    *,
    changes: list[dict[str, Any]],
    patchsets_by_id: dict[str, dict[str, Any]],
    inventory_by_path: dict[str, list[dict[str, Any]]],
    inventory_by_function: dict[str, list[dict[str, Any]]],
) -> dict[str, Any]:
    snapshot_cases_by_id: dict[str, dict[str, Any]] = {}
    source_change_ids: dict[str, set[str]] = defaultdict(set)
    source_patchset_ids: dict[str, set[str]] = defaultdict(set)
    changed_test_files: set[str] = set()
    missing_test_file_paths: set[str] = set()
    unresolved_reasons: list[str] = []

    if not changes:
        unresolved_reasons.append("no_remote_changes_for_task")

    snapshot_basis_count = 0
    for change in changes:
        patchset_id = _normalize_text(change.get("selected_patchset_id")) or _normalize_text(change.get("current_patchset_id"))
        if not patchset_id:
            unresolved_reasons.append(f"change:{change['change_id']}:no_patchset_snapshot_basis")
            continue
        patchset = patchsets_by_id.get(patchset_id)
        if patchset is None:
            unresolved_reasons.append(f"change:{change['change_id']}:patchset_missing_from_inventory")
            continue
        snapshot_basis_count += 1
        for path in extract_changed_test_files(patchset.get("diff_stats")):
            changed_test_files.add(path)
            path_rows = inventory_by_path.get(path, [])
            if not path_rows:
                missing_test_file_paths.add(path)
                continue
            for row in path_rows:
                test_case_id = str(row["test_case_id"])
                snapshot_cases_by_id[test_case_id] = dict(row)
                source_change_ids[test_case_id].add(str(change["change_id"]))
                source_patchset_ids[test_case_id].add(str(patchset_id))

    if changes and snapshot_basis_count == 0:
        unresolved_reasons.append("no_snapshot_backed_patchsets_for_task")
    if missing_test_file_paths:
        unresolved_reasons.append("missing_test_file_inventory")

    snapshot_cases = _unique_case_rows(snapshot_cases_by_id.values())
    function_names = sorted({str(row["function_name"]) for row in snapshot_cases})
    reviewer_rows = _unique_case_rows(
        row
        for function_name in function_names
        for row in inventory_by_function.get(function_name, [])
    )
    snapshot_case_ids = {str(row["test_case_id"]) for row in snapshot_cases}
    reviewer_case_ids = {str(row["test_case_id"]) for row in reviewer_rows}
    counts_match = snapshot_case_ids == reviewer_case_ids and not unresolved_reasons

    link_rows: list[dict[str, Any]] = []
    if counts_match:
        for row in snapshot_cases:
            test_case_id = str(row["test_case_id"])
            link_rows.append(
                {
                    "repo_name": str(task["repo_name"]),
                    "repo_id": str(task["repo_id"]),
                    "task_id": str(task["task_id"]),
                    "test_case_id": test_case_id,
                    "pytest_node_id": str(row["pytest_node_id"]),
                    "test_file_path": str(row["test_file_path"]),
                    "class_name": row.get("class_name"),
                    "function_name": str(row["function_name"]),
                    "source_line": int(row["source_line"]),
                    "source_change_ids": sorted(source_change_ids.get(test_case_id, set())),
                    "source_patchset_ids": sorted(source_patchset_ids.get(test_case_id, set())),
                    "verification_mode": "snapshot_diff_paths_then_function_name_reverse_lookup",
                }
            )

    return {
        "task_id": str(task["task_id"]),
        "title": str(task["title"]),
        "status": str(task["status"]),
        "change_count": len(changes),
        "snapshot_backed_change_count": snapshot_basis_count,
        "changed_test_files": sorted(changed_test_files),
        "missing_test_file_paths": sorted(missing_test_file_paths),
        "snapshot_test_case_count": len(snapshot_case_ids),
        "reviewer_test_case_count": len(reviewer_case_ids),
        "counts_match": counts_match,
        "unresolved_reasons": sorted(set(unresolved_reasons)),
        "function_names": function_names,
        "source_change_ids": sorted({str(change["change_id"]) for change in changes}),
        "source_patchset_ids": sorted({patchset_id for rows in source_patchset_ids.values() for patchset_id in rows}),
        "link_rows": link_rows,
    }


def _insert_task_test_case_links(cursor: Any, *, control_schema: str, repo_id: str, rows: list[dict[str, Any]], linked_at: str) -> None:
    cursor.execute(f'set search_path to "{control_schema}", public')
    cursor.execute(f"delete from {LINK_TABLE} where repo_id = %s", (repo_id,))
    for row in rows:
        cursor.execute(
            f"""
            insert into {LINK_TABLE} (
                repo_name, repo_id, task_id, test_case_id, pytest_node_id, test_file_path, class_name,
                function_name, source_line, source_change_ids_json, source_patchset_ids_json, verification_mode, linked_at
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
                row["repo_name"],
                row["repo_id"],
                row["task_id"],
                row["test_case_id"],
                row["pytest_node_id"],
                row["test_file_path"],
                row.get("class_name"),
                row["function_name"],
                row["source_line"],
                json.dumps(row["source_change_ids"], ensure_ascii=False),
                json.dumps(row["source_patchset_ids"], ensure_ascii=False),
                row["verification_mode"],
                linked_at,
            ),
        )


def _write_output(path: Path | None, text: str) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text + ("" if text.endswith("\n") else "\n"), encoding="utf-8")


def _build_mismatch_markdown(payload: dict[str, Any]) -> str:
    lines = [
        f"# Task Test Case Link Mismatches for {payload['repo_name']} ({payload['repo_id']})",
        "",
        f"- generated_at: {payload['generated_at']}",
        f"- total_tasks: {payload['summary']['task_count']}",
        f"- mismatch_task_count: {payload['summary']['mismatch_task_count']}",
        f"- unresolved_task_count: {payload['summary']['unresolved_task_count']}",
        "",
    ]
    for row in payload["mismatches"]:
        lines.append(f"## {row['task_id']} {row['title']}")
        lines.append(f"- status: {row['status']}")
        lines.append(f"- change_count: {row['change_count']}")
        lines.append(f"- snapshot_backed_change_count: {row['snapshot_backed_change_count']}")
        lines.append(f"- changed_test_file_count: {len(row['changed_test_files'])}")
        lines.append(f"- snapshot_test_case_count: {row['snapshot_test_case_count']}")
        lines.append(f"- reviewer_test_case_count: {row['reviewer_test_case_count']}")
        lines.append(f"- unresolved_reasons: {', '.join(row['unresolved_reasons']) if row['unresolved_reasons'] else 'none'}")
        if row["missing_test_file_paths"]:
            lines.append(f"- missing_test_file_paths: {', '.join(row['missing_test_file_paths'])}")
        lines.append("")
    return "\n".join(lines)


def sync_remote_task_test_case_links(
    *,
    dsn: str,
    repo_root: Path,
    remote_name: str,
    control_schema: str,
    content_schema: str,
    output_path: Path | None,
    mismatch_json_path: Path | None,
    mismatch_markdown_path: Path | None,
) -> dict[str, Any]:
    psycopg = _load_psycopg()
    normalized_control_schema = _normalize_schema_name(control_schema, field="control_schema")
    normalized_content_schema = _normalize_schema_name(content_schema, field="content_schema")
    ctx, repo_name = _resolve_repo_context(repo_root)
    remote_url = _resolve_remote_url(ctx, remote_name)

    tasks = remote_client.list_tasks(remote_url, repo_name)
    if not tasks:
        raise RuntimeError(f"No remote tasks found for repo {repo_name!r} on remote {remote_name!r}")
    repo_id = _normalize_text(tasks[0].get("repo_id"))
    if not repo_id:
        raise RuntimeError("Remote tasks are missing repo_id; cannot build repo-scoped links.")

    inventory_module = _load_inventory_script_module()
    inventory_sync = inventory_module.sync_test_case_inventory(
        dsn=dsn,
        repo_root=repo_root,
        tests_root=repo_root / "tests",
        repo_name=repo_name,
        repo_id=repo_id,
        content_schema=normalized_content_schema,
        control_schema=normalized_control_schema,
    )

    synced_at = _now_utc().isoformat()
    changes = remote_client.list_changes(remote_url, repo_name)

    change_ids = [str(row["change_id"]) for row in changes]
    change_details: list[dict[str, Any]] = []
    with ThreadPoolExecutor(max_workers=min(8, max(len(change_ids), 1))) as executor:
        future_map = {
            executor.submit(remote_client.get_change, remote_url, change_id, repo_name=repo_name): change_id
            for change_id in change_ids
        }
        for future in as_completed(future_map):
            change_details.append(future.result())
    change_details.sort(key=lambda row: (int(row.get("change_seq") or 0), str(row["change_id"])))

    selected_patchset_pairs = []
    for row in change_details:
        patchset_id = _normalize_text(row.get("selected_patchset_id")) or _normalize_text(row.get("current_patchset_id"))
        if patchset_id:
            selected_patchset_pairs.append((str(row["change_id"]), patchset_id))
    seen_patchset_ids: set[str] = set()
    unique_patchset_pairs: list[tuple[str, str]] = []
    for change_id, patchset_id in selected_patchset_pairs:
        if patchset_id in seen_patchset_ids:
            continue
        seen_patchset_ids.add(patchset_id)
        unique_patchset_pairs.append((change_id, patchset_id))

    patchsets: list[dict[str, Any]] = []
    with ThreadPoolExecutor(max_workers=min(8, max(len(unique_patchset_pairs), 1))) as executor:
        future_map = {
            executor.submit(remote_client.get_patchset, remote_url, patchset_id, repo_name=repo_name, change_ref=change_id): (change_id, patchset_id)
            for change_id, patchset_id in unique_patchset_pairs
        }
        for future in as_completed(future_map):
            patchsets.append(future.result())
    patchsets.sort(key=lambda row: (str(row["change_id"]), int(row.get("patchset_number") or 0), str(row["patchset_id"])))

    legacy_removed = False
    if LEGACY_JSON_PATH.exists():
        LEGACY_JSON_PATH.unlink()
        legacy_removed = True

    with closing(psycopg.connect(dsn, row_factory=psycopg.rows.dict_row)) as connection:
        with connection.cursor() as cursor:
            _ensure_supporting_tables(cursor, control_schema=normalized_control_schema)
            _upsert_remote_tasks(cursor, control_schema=normalized_control_schema, rows=tasks, synced_at=synced_at)
            _upsert_remote_changes(cursor, control_schema=normalized_control_schema, rows=change_details, synced_at=synced_at)
            _upsert_remote_patchsets(
                cursor,
                control_schema=normalized_control_schema,
                rows=patchsets,
                repo_name=repo_name,
                repo_id=repo_id,
                synced_at=synced_at,
            )
            inventory_by_path, inventory_by_function = _load_inventory_rows(
                cursor,
                control_schema=normalized_control_schema,
                repo_id=repo_id,
            )
            db_tasks, db_changes, db_patchsets = _load_remote_db_rows(
                cursor,
                control_schema=normalized_control_schema,
                repo_id=repo_id,
            )
            changes_by_task: dict[str, list[dict[str, Any]]] = defaultdict(list)
            for row in db_changes:
                changes_by_task[str(row["task_id"])].append(row)

            verification_rows = [
                build_task_verification(
                    task,
                    changes=changes_by_task.get(str(task["task_id"]), []),
                    patchsets_by_id=db_patchsets,
                    inventory_by_path=inventory_by_path,
                    inventory_by_function=inventory_by_function,
                )
                for task in db_tasks
            ]

            matched_rows = [row for row in verification_rows if row["counts_match"]]
            mismatched_rows = [row for row in verification_rows if not row["counts_match"]]
            link_rows = [link for row in matched_rows for link in row["link_rows"]]
            _insert_task_test_case_links(
                cursor,
                control_schema=normalized_control_schema,
                repo_id=repo_id,
                rows=link_rows,
                linked_at=synced_at,
            )
        connection.commit()

    summary = {
        "task_count": len(tasks),
        "change_count": len(change_details),
        "patchset_count": len(patchsets),
        "matched_task_count": len(matched_rows),
        "mismatch_task_count": len(mismatched_rows),
        "unresolved_task_count": sum(1 for row in verification_rows if row["unresolved_reasons"]),
        "tasks_with_link_rows": sum(1 for row in matched_rows if row["link_rows"]),
        "inserted_link_count": len(link_rows),
        "verified_zero_test_case_task_count": sum(
            1
            for row in matched_rows
            if row["snapshot_test_case_count"] == 0 and row["reviewer_test_case_count"] == 0
        ),
        "legacy_176_json_removed": legacy_removed,
    }
    payload = {
        "generated_at": synced_at,
        "repo_name": repo_name,
        "repo_id": repo_id,
        "remote_name": remote_name,
        "remote_url": remote_url,
        "inventory_sync": inventory_sync,
        "summary": summary,
        "matches": matched_rows,
        "mismatches": mismatched_rows,
    }

    _write_output(output_path, _json_dump(payload))
    _write_output(mismatch_json_path, _json_dump({"generated_at": synced_at, "repo_name": repo_name, "repo_id": repo_id, "summary": summary, "mismatches": mismatched_rows}))
    _write_output(mismatch_markdown_path, _build_mismatch_markdown(payload))
    return payload


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dsn", default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN"), help="PostgreSQL DSN.")
    parser.add_argument("--repo-root", default=".", help="Repository root. Defaults to the current directory.")
    parser.add_argument("--remote", default="origin", help="Remote name. Defaults to origin.")
    parser.add_argument(
        "--content-schema",
        default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTENT_SCHEMA", "ait_native_content"),
        help="PostgreSQL content schema.",
    )
    parser.add_argument(
        "--control-schema",
        default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_CONTROL_SCHEMA", "ait_native_control"),
        help="PostgreSQL control schema.",
    )
    parser.add_argument("--json", action="store_true", help="Emit JSON.")
    parser.add_argument("--output", help="Optional JSON output path.")
    parser.add_argument("--mismatch-json", help="Optional mismatch JSON output path.")
    parser.add_argument("--mismatch-markdown", help="Optional mismatch Markdown output path.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    dsn = str(args.dsn or "").strip()
    if not dsn:
        parser.error("Pass --dsn or set AIT_NATIVE_SERVER_POSTGRES_DSN.")
    payload = sync_remote_task_test_case_links(
        dsn=dsn,
        repo_root=Path(args.repo_root).expanduser().resolve(),
        remote_name=str(args.remote),
        control_schema=str(args.control_schema),
        content_schema=str(args.content_schema),
        output_path=Path(args.output).expanduser() if args.output else None,
        mismatch_json_path=Path(args.mismatch_json).expanduser() if args.mismatch_json else None,
        mismatch_markdown_path=Path(args.mismatch_markdown).expanduser() if args.mismatch_markdown else None,
    )
    if not args.output:
        print(_json_dump(payload) if args.json or True else payload)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
