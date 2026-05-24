#!/usr/bin/env python3
"""Render the Wave 1 PostgreSQL repositories-table rebuild rehearsal plan."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


DEFAULT_ACTIVE_DSN_ENV = "AIT_NATIVE_SERVER_POSTGRES_DSN"
DEFAULT_ROLLBACK_DSN_ENV = "AIT_ROLLBACK_POSTGRES_DSN"
DEFAULT_EXPORT_PATH = "repositories_wave1.csv"

COLUMNS = [
    "repo_id",
    "repo_name",
    "default_line",
    "id_namespace_prefix",
    "policy_json",
    "created_at",
    "updated_at",
]


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _qualified_table(schema: str, table: str) -> str:
    return f'"{schema}"."{table}"'


def build_repositories_rebuild_plan(
    *,
    rollback_dsn_env: str = DEFAULT_ROLLBACK_DSN_ENV,
    active_dsn_env: str = DEFAULT_ACTIVE_DSN_ENV,
    source_schema: str = "public",
    target_schema: str = "public",
    source_table: str = "repositories",
    target_table: str = "repositories",
    export_path: str = DEFAULT_EXPORT_PATH,
) -> dict[str, Any]:
    source_relation = _qualified_table(source_schema, source_table)
    target_relation = _qualified_table(target_schema, target_table)
    column_list = ", ".join(COLUMNS)
    export_query = (
        f"select {column_list} from {source_relation} "
        "where repo_id is not null order by created_at, repo_name"
    )
    export_command = (
        f"psql \"${rollback_dsn_env}\" -v ON_ERROR_STOP=1 -c "
        f"\\\"\\copy ({export_query}) to '{export_path}' csv header\\\""
    )
    create_table_sql = "\n".join(
        [
            f"drop table if exists {target_relation} cascade;",
            "",
            f"create table {target_relation} (",
            "    repo_id text primary key,",
            "    repo_name text not null unique,",
            "    default_line text not null,",
            "    id_namespace_prefix text not null default 'AIT',",
            "    policy_json text not null default '{}',",
            "    created_at timestamptz not null,",
            "    updated_at timestamptz not null",
            ");",
        ]
    )
    import_command = (
        f"psql \"${active_dsn_env}\" -v ON_ERROR_STOP=1 -c "
        f"\\\"\\copy {target_relation} ({column_list}) from '{export_path}' csv header\\\""
    )
    validation_queries = [
        f"select count(*) as repo_count from {target_relation};",
        f"select count(*) as null_repo_id_count from {target_relation} where repo_id is null;",
        f"select repo_name, count(*) as duplicates from {target_relation} group by repo_name having count(*) > 1;",
        f"select repo_id, count(*) as duplicates from {target_relation} group by repo_id having count(*) > 1;",
        f"select id_namespace_prefix, count(*) as duplicates from {target_relation} group by id_namespace_prefix having count(*) > 1;",
    ]
    source_preflight_queries = [
        f"select count(*) as repo_count from {source_relation};",
        f"select repo_name from {source_relation} where repo_id is null or repo_id = '' order by repo_name;",
    ]
    follow_up_notes = [
        "Run this helper after Wave 0 freeze, backup, and rollback-source capture are complete.",
        "The helper renders a rehearsal plan; it does not execute the rebuild automatically.",
        "Use the immutable rollback database copy as the export source rather than trying to select across PostgreSQL databases inside one SQL statement.",
        "Recreate repository-foundation compatibility indexes such as the namespace-prefix unique index in the separate `foundation-indexes` slice.",
    ]
    return {
        "active_dsn_env": active_dsn_env,
        "rollback_dsn_env": rollback_dsn_env,
        "source_relation": source_relation,
        "target_relation": target_relation,
        "export_path": export_path,
        "columns": COLUMNS,
        "source_preflight_queries": source_preflight_queries,
        "export_query": export_query,
        "export_command": export_command,
        "create_table_sql": create_table_sql,
        "import_command": import_command,
        "validation_queries": validation_queries,
        "follow_up_notes": follow_up_notes,
    }


def format_plan(payload: dict[str, Any]) -> str:
    lines = [
        f"Rollback source: ${payload['rollback_dsn_env']} -> {payload['source_relation']}",
        f"Active target: ${payload['active_dsn_env']} -> {payload['target_relation']}",
        f"Export path: {payload['export_path']}",
        "",
        "Source preflight queries:",
    ]
    for query in payload["source_preflight_queries"]:
        lines.append(query)
    lines.extend(
        [
            "",
            "Rollback export command:",
            payload["export_command"],
            "",
            "Active rebuild DDL:",
            payload["create_table_sql"],
            "",
            "Active import command:",
            payload["import_command"],
            "",
            "Validation queries:",
        ]
    )
    lines.extend(payload["validation_queries"])
    lines.extend(["", "Notes:"])
    for note in payload["follow_up_notes"]:
        lines.append(f"- {note}")
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rollback-dsn-env", default=DEFAULT_ROLLBACK_DSN_ENV, help=f"Environment variable name that holds the rollback database DSN. Default: {DEFAULT_ROLLBACK_DSN_ENV}.")
    parser.add_argument("--active-dsn-env", default=DEFAULT_ACTIVE_DSN_ENV, help=f"Environment variable name that holds the active database DSN. Default: {DEFAULT_ACTIVE_DSN_ENV}.")
    parser.add_argument("--source-schema", default="public", help="Rollback/source schema name. Default: public.")
    parser.add_argument("--target-schema", default="public", help="Active target schema name. Default: public.")
    parser.add_argument("--source-table", default="repositories", help="Rollback/source table name. Default: repositories.")
    parser.add_argument("--target-table", default="repositories", help="Active target table name. Default: repositories.")
    parser.add_argument("--export-path", default=DEFAULT_EXPORT_PATH, help=f"CSV export path used in the rendered commands. Default: {DEFAULT_EXPORT_PATH}.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of text.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    payload = build_repositories_rebuild_plan(
        rollback_dsn_env=args.rollback_dsn_env,
        active_dsn_env=args.active_dsn_env,
        source_schema=args.source_schema,
        target_schema=args.target_schema,
        source_table=args.source_table,
        target_table=args.target_table,
        export_path=args.export_path,
    )
    text = _json_dump(payload) if args.json else format_plan(payload)
    if args.output:
        output_path = Path(args.output).expanduser()
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(text + ("" if text.endswith("\n") else "\n"), encoding="utf-8")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
