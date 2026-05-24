#!/usr/bin/env python3
"""Render the Wave 2 PostgreSQL lines-table rebuild rehearsal plan."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

DEFAULT_ACTIVE_DSN_ENV = "AIT_NATIVE_SERVER_POSTGRES_DSN"
DEFAULT_ROLLBACK_DSN_ENV = "AIT_ROLLBACK_POSTGRES_DSN"
DEFAULT_EXPORT_PATH = "lines_wave2.csv"
LINES_REPO_ID_INDEX = "idx_lines_repo_id"
LINES_REPO_COMPAT_INDEX = "idx_lines_repo"
COLUMNS = ["repo_name", "repo_id", "line_name", "status", "archived_at", "created_at", "updated_at"]


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _qualified_table(schema: str, table: str) -> str:
    return f'"{schema}"."{table}"'


def build_lines_rebuild_plan(
    *,
    rollback_dsn_env: str = DEFAULT_ROLLBACK_DSN_ENV,
    active_dsn_env: str = DEFAULT_ACTIVE_DSN_ENV,
    source_schema: str = "public",
    target_schema: str = "public",
    source_table: str = "lines",
    target_table: str = "lines",
    repositories_table: str = "repositories",
    export_path: str = DEFAULT_EXPORT_PATH,
) -> dict[str, Any]:
    source_lines_relation = _qualified_table(source_schema, source_table)
    source_repositories_relation = _qualified_table(source_schema, repositories_table)
    target_lines_relation = _qualified_table(target_schema, target_table)
    target_repositories_relation = _qualified_table(target_schema, repositories_table)
    column_list = ", ".join(COLUMNS)
    export_query = "\n".join(
        [
            "select l.repo_name, coalesce(l.repo_id, r.repo_id) as repo_id, l.line_name, l.status, l.archived_at, l.created_at, l.updated_at",
            f"from {source_lines_relation} as l",
            f"join {source_repositories_relation} as r on r.repo_name = l.repo_name",
            "order by l.repo_name, l.line_name",
        ]
    )
    export_command = (
        f"psql \"${rollback_dsn_env}\" -v ON_ERROR_STOP=1 -c "
        f"\\\"\\copy ({export_query}) to '{export_path}' csv header\\\""
    )
    create_table_sql = "\n".join(
        [
            f"drop table if exists {target_lines_relation} cascade;",
            "",
            f"create table {target_lines_relation} (",
            f"    repo_name text not null references {target_repositories_relation}(repo_name) on delete cascade,",
            f"    repo_id text not null references {target_repositories_relation}(repo_id) on delete cascade,",
            "    line_name text not null,",
            "    status text not null default 'active',",
            "    archived_at timestamptz,",
            "    created_at timestamptz not null,",
            "    updated_at timestamptz not null,",
            "    primary key (repo_id, line_name)",
            ");",
        ]
    )
    post_create_index_sql = [
        f"create index if not exists {LINES_REPO_COMPAT_INDEX} on {target_lines_relation}(repo_name, line_name);",
        f"create index if not exists {LINES_REPO_ID_INDEX} on {target_lines_relation}(repo_id, line_name);",
    ]
    import_command = (
        f"psql \"${active_dsn_env}\" -v ON_ERROR_STOP=1 -c "
        f"\\\"\\copy {target_lines_relation} ({column_list}) from '{export_path}' csv header\\\""
    )
    source_preflight_queries = [
        f"select count(*) as line_count from {source_lines_relation};",
        "\n".join(
            [
                "select l.repo_name, l.line_name",
                f"from {source_lines_relation} as l",
                f"left join {source_repositories_relation} as r on r.repo_name = l.repo_name",
                "where r.repo_name is null",
                "order by l.repo_name, l.line_name;",
            ]
        ),
        "\n".join(
            [
                "select l.repo_name, l.line_name",
                f"from {source_lines_relation} as l",
                "where l.repo_id is null or btrim(l.repo_id) = ''",
                "order by l.repo_name, l.line_name;",
            ]
        ),
    ]
    validation_queries = [
        f"select count(*) as line_count from {target_lines_relation};",
        f"select count(*) as null_repo_id_count from {target_lines_relation} where repo_id is null or btrim(repo_id) = '';",
        "\n".join(
            [
                "select repo_id, line_name, count(*) as duplicates",
                f"from {target_lines_relation}",
                "group by repo_id, line_name",
                "having count(*) > 1;",
            ]
        ),
        "\n".join(
            [
                "select l.repo_name, l.repo_id as line_repo_id, r.repo_id as repository_repo_id",
                f"from {target_lines_relation} as l",
                f"join {target_repositories_relation} as r on r.repo_name = l.repo_name",
                "where l.repo_id is distinct from r.repo_id",
                "order by l.repo_name, l.line_name;",
            ]
        ),
        "\n".join(
            [
                "select indexname",
                "from pg_indexes",
                f"where schemaname = '{target_schema}'",
                f"  and tablename = '{target_table}'",
                f"  and indexname in ('{LINES_REPO_COMPAT_INDEX}', '{LINES_REPO_ID_INDEX}')",
                "order by indexname;",
            ]
        ),
    ]
    follow_up_notes = [
        "Run this helper after the Wave 1 repository foundation slices land and while the cutover still stays inside the frozen-write window.",
        "Keep repo_id as the canonical line authority, but retain repo_name temporarily for operator readability and legacy ref-path compatibility.",
        "Do not remove the repo_name compatibility index until downstream readers and routing surfaces no longer depend on repo_name-scoped line lookup.",
        "The snapshots and packs rebuild slice should assume the lines table already exposes canonical repo_id lineage after this step.",
    ]
    return {
        "active_dsn_env": active_dsn_env,
        "rollback_dsn_env": rollback_dsn_env,
        "source_lines_relation": source_lines_relation,
        "source_repositories_relation": source_repositories_relation,
        "target_lines_relation": target_lines_relation,
        "target_repositories_relation": target_repositories_relation,
        "export_path": export_path,
        "columns": COLUMNS,
        "source_preflight_queries": source_preflight_queries,
        "export_query": export_query,
        "export_command": export_command,
        "create_table_sql": create_table_sql,
        "post_create_index_sql": post_create_index_sql,
        "import_command": import_command,
        "validation_queries": validation_queries,
        "follow_up_notes": follow_up_notes,
    }


def format_plan(payload: dict[str, Any]) -> str:
    lines = [
        f"Rollback source: ${payload['rollback_dsn_env']} -> {payload['source_lines_relation']}",
        f"Active target: ${payload['active_dsn_env']} -> {payload['target_lines_relation']}",
        f"Export path: {payload['export_path']}",
        "",
        "Source preflight queries:",
    ]
    lines.extend(payload["source_preflight_queries"])
    lines.extend(["", "Rollback export command:", payload["export_command"], "", "Active rebuild DDL:", payload["create_table_sql"], "", "Post-create index SQL:"])
    lines.extend(payload["post_create_index_sql"])
    lines.extend(["", "Active import command:", payload["import_command"], "", "Validation queries:"])
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
    parser.add_argument("--source-table", default="lines", help="Rollback/source lines table name. Default: lines.")
    parser.add_argument("--target-table", default="lines", help="Active target lines table name. Default: lines.")
    parser.add_argument("--repositories-table", default="repositories", help="Repositories table name in both source and target schemas. Default: repositories.")
    parser.add_argument("--export-path", default=DEFAULT_EXPORT_PATH, help=f"CSV export path used in the rendered commands. Default: {DEFAULT_EXPORT_PATH}.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of text.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    payload = build_lines_rebuild_plan(
        rollback_dsn_env=args.rollback_dsn_env,
        active_dsn_env=args.active_dsn_env,
        source_schema=args.source_schema,
        target_schema=args.target_schema,
        source_table=args.source_table,
        target_table=args.target_table,
        repositories_table=args.repositories_table,
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
