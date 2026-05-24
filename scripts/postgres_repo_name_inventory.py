#!/usr/bin/env python3
"""Capture Wave 0 PostgreSQL repo_name dependency inventory for repo_id cutover."""

from __future__ import annotations

import argparse
import importlib
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

SYSTEM_SCHEMAS = ("pg_catalog", "information_schema")

WAVE_GROUPS: tuple[tuple[str, str, tuple[str, ...]], ...] = (
    (
        "wave_1_repository_foundation",
        "Wave 1: repository foundation rebuild",
        ("repositories", "repository_group_memberships"),
    ),
    (
        "wave_2_content_plane",
        "Wave 2: content-plane repo-scoped tables",
        ("lines", "snapshots", "packs"),
    ),
    (
        "wave_3_workflow_roots",
        "Wave 3: workflow roots",
        (
            "plans",
            "tasks",
            "changes",
            "releases",
            "sessions",
            "planning_sessions",
            "stacks",
            "role_bindings",
            "jobs",
            "authority_maps",
        ),
    ),
    (
        "wave_4_workflow_children",
        "Wave 4: workflow children and parent-bound lineage",
        (
            "plan_revisions",
            "plan_revision_blobs",
            "plan_revision_artifacts",
            "planning_session_events",
            "session_events",
            "session_checkpoints",
            "patchsets",
            "review_requests",
            "reviews",
            "attestations",
            "policy_decisions",
            "waivers",
            "land_requests",
            "stack_changes",
            "authority_nodes",
            "authority_mutations",
        ),
    ),
)

TABLE_TO_WAVE: dict[str, tuple[str, str]] = {
    table_name: (wave, label)
    for wave, label, table_names in WAVE_GROUPS
    for table_name in table_names
}

REPO_NAME_COLUMNS_SQL = """
select table_schema, table_name, column_name, data_type
from information_schema.columns
where table_schema not in ('pg_catalog', 'information_schema')
  and column_name = 'repo_name'
order by table_schema, table_name, column_name;
""".strip()

REPO_NAME_CONSTRAINTS_SQL = """
select ns.nspname as table_schema,
       cls.relname as table_name,
       con.conname as constraint_name,
       pg_get_constraintdef(con.oid) as definition
from pg_constraint con
join pg_class cls on cls.oid = con.conrelid
join pg_namespace ns on ns.oid = cls.relnamespace
where con.contype in ('f', 'p', 'u')
  and ns.nspname not in ('pg_catalog', 'information_schema')
  and pg_get_constraintdef(con.oid) like '%repo_name%'
order by ns.nspname, cls.relname, con.conname;
""".strip()

REPO_NAME_INDEXES_SQL = """
select schemaname, tablename, indexname, indexdef
from pg_indexes
where schemaname not in ('pg_catalog', 'information_schema')
  and indexdef like '%repo_name%'
order by schemaname, tablename, indexname;
""".strip()


def _now_utc() -> datetime:
    return datetime.now(timezone.utc)


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _table_ref(schema: str, table: str) -> str:
    return f"{schema}.{table}"


def _wave_for_table(table_name: str) -> tuple[str | None, str | None]:
    return TABLE_TO_WAVE.get(table_name, (None, None))


def _load_psycopg():
    try:
        return importlib.import_module("psycopg")
    except ModuleNotFoundError as exc:
        raise RuntimeError(
            "PostgreSQL inventory requires psycopg. Install with: pip install 'ait-native[postgres]'"
        ) from exc


def _column_names(description: Any) -> list[str]:
    columns: list[str] = []
    for index, row in enumerate(description or []):
        name = getattr(row, "name", None)
        if name is None and isinstance(row, (list, tuple)) and row:
            name = row[0]
        columns.append(str(name or f"column_{index}"))
    return columns


def _query_rows(cursor: Any, sql: str) -> list[dict[str, Any]]:
    cursor.execute(sql)
    columns = _column_names(getattr(cursor, "description", None))
    return [dict(zip(columns, values)) for values in cursor.fetchall()]


def summarize_inventory(
    repo_name_columns: list[dict[str, Any]],
    repo_name_constraints: list[dict[str, Any]],
    repo_name_indexes: list[dict[str, Any]],
) -> dict[str, Any]:
    table_rows: dict[tuple[str, str], dict[str, Any]] = {}

    def ensure_table(schema: str, table: str) -> dict[str, Any]:
        key = (schema, table)
        row = table_rows.get(key)
        if row is None:
            wave, wave_label = _wave_for_table(table)
            row = {
                "table_schema": schema,
                "table_name": table,
                "table_ref": _table_ref(schema, table),
                "wave": wave,
                "wave_label": wave_label,
                "repo_name_columns": [],
                "repo_name_constraints": [],
                "repo_name_indexes": [],
            }
            table_rows[key] = row
        return row

    for row in repo_name_columns:
        table = ensure_table(str(row["table_schema"]), str(row["table_name"]))
        column_name = str(row["column_name"])
        if column_name not in table["repo_name_columns"]:
            table["repo_name_columns"].append(column_name)

    for row in repo_name_constraints:
        table = ensure_table(str(row["table_schema"]), str(row["table_name"]))
        constraint_name = str(row["constraint_name"])
        if constraint_name not in table["repo_name_constraints"]:
            table["repo_name_constraints"].append(constraint_name)

    for row in repo_name_indexes:
        table = ensure_table(str(row["schemaname"]), str(row["tablename"]))
        index_name = str(row["indexname"])
        if index_name not in table["repo_name_indexes"]:
            table["repo_name_indexes"].append(index_name)

    table_summary = sorted(
        table_rows.values(),
        key=lambda row: (
            next((index for index, (wave, _, _) in enumerate(WAVE_GROUPS) if wave == row["wave"]), len(WAVE_GROUPS)),
            row["table_schema"],
            row["table_name"],
        ),
    )
    for row in table_summary:
        row["repo_name_columns"].sort()
        row["repo_name_constraints"].sort()
        row["repo_name_indexes"].sort()
        row["dependency_count"] = (
            len(row["repo_name_columns"])
            + len(row["repo_name_constraints"])
            + len(row["repo_name_indexes"])
        )

    wave_groups: list[dict[str, Any]] = []
    for wave, label, _table_names in WAVE_GROUPS:
        tables = [row["table_ref"] for row in table_summary if row["wave"] == wave]
        wave_groups.append({"wave": wave, "label": label, "tables": tables})

    unclassified_tables = [row["table_ref"] for row in table_summary if row["wave"] is None]
    return {
        "table_summary": table_summary,
        "wave_groups": wave_groups,
        "unclassified_tables": unclassified_tables,
    }


def capture_live_inventory(dsn: str) -> dict[str, Any]:
    psycopg = _load_psycopg()
    with psycopg.connect(dsn) as connection:
        with connection.cursor() as cursor:
            repo_name_columns = _query_rows(cursor, REPO_NAME_COLUMNS_SQL)
            repo_name_constraints = _query_rows(cursor, REPO_NAME_CONSTRAINTS_SQL)
            repo_name_indexes = _query_rows(cursor, REPO_NAME_INDEXES_SQL)
    summary = summarize_inventory(repo_name_columns, repo_name_constraints, repo_name_indexes)
    return {
        "generated_at": _now_utc().isoformat(),
        "repo_name_columns": repo_name_columns,
        "repo_name_constraints": repo_name_constraints,
        "repo_name_indexes": repo_name_indexes,
        **summary,
    }


def format_inventory_report(payload: dict[str, Any]) -> str:
    lines = [f"Generated at: {payload['generated_at']}"]
    table_lookup = {
        str(row["table_ref"]): row for row in list(payload.get("table_summary") or []) if isinstance(row, dict)
    }
    emitted_any = False
    for group in list(payload.get("wave_groups") or []):
        tables = [str(row) for row in list(group.get("tables") or [])]
        if not tables:
            continue
        emitted_any = True
        lines.append("")
        lines.append(f"{group['label']} ({group['wave']})")
        for table_ref in tables:
            row = table_lookup.get(table_ref, {})
            lines.append(f"- {table_ref}")
            if row.get("repo_name_columns"):
                lines.append(f"  columns: {', '.join(row['repo_name_columns'])}")
            if row.get("repo_name_constraints"):
                lines.append(f"  constraints: {', '.join(row['repo_name_constraints'])}")
            if row.get("repo_name_indexes"):
                lines.append(f"  indexes: {', '.join(row['repo_name_indexes'])}")
    unclassified_tables = [str(row) for row in list(payload.get("unclassified_tables") or [])]
    if unclassified_tables:
        emitted_any = True
        lines.append("")
        lines.append("Unclassified tables")
        for table_ref in unclassified_tables:
            lines.append(f"- {table_ref}")
    if not emitted_any:
        lines.append("")
        lines.append("No repo_name-scoped dependencies found.")
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--dsn",
        default=os.environ.get("AIT_NATIVE_SERVER_POSTGRES_DSN"),
        help="PostgreSQL DSN. Defaults to AIT_NATIVE_SERVER_POSTGRES_DSN when set.",
    )
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of a text summary.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    dsn = str(args.dsn or "").strip()
    if not dsn:
        parser.error("Pass --dsn or set AIT_NATIVE_SERVER_POSTGRES_DSN.")
    try:
        payload = capture_live_inventory(dsn)
    except Exception as exc:
        print(_json_dump({"ok": False, "error": str(exc)}), file=sys.stderr)
        return 1

    text = _json_dump(payload) if args.json else format_inventory_report(payload)
    if args.output:
        output_path = Path(args.output).expanduser()
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(text + ("" if text.endswith("\n") else "\n"), encoding="utf-8")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
