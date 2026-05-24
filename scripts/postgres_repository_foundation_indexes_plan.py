#!/usr/bin/env python3
"""Render the Wave 1 PostgreSQL foundation-indexes rehearsal plan."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

DEFAULT_ACTIVE_DSN_ENV = "AIT_NATIVE_SERVER_POSTGRES_DSN"
DEFAULT_SCHEMA = "public"
DEFAULT_REPOSITORIES_TABLE = "repositories"
DEFAULT_MEMBERSHIPS_TABLE = "repository_group_memberships"
REPOSITORY_ID_UNIQUE_INDEX = "idx_repositories_repo_id_unique"
REPOSITORY_NAMESPACE_PREFIX_UNIQUE_INDEX = "idx_repositories_namespace_prefix_unique"
REPOSITORY_GROUP_MEMBERSHIPS_REPO_ID_UNIQUE_INDEX = "idx_repository_group_memberships_repo_id_unique"
REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_ID_INDEX = "idx_repository_group_memberships_group_repo_id"
REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_NAME_INDEX = "idx_repository_group_memberships_group"


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _qualified_table(schema: str, table: str) -> str:
    return f'"{schema}"."{table}"'


def build_foundation_indexes_plan(
    *,
    active_dsn_env: str = DEFAULT_ACTIVE_DSN_ENV,
    schema: str = DEFAULT_SCHEMA,
    repositories_table: str = DEFAULT_REPOSITORIES_TABLE,
    memberships_table: str = DEFAULT_MEMBERSHIPS_TABLE,
) -> dict[str, Any]:
    repositories_relation = _qualified_table(schema, repositories_table)
    memberships_relation = _qualified_table(schema, memberships_table)
    compatibility_column_sql = [
        f"alter table {repositories_relation} add column if not exists id_namespace_prefix text not null default 'AIT';",
        f"alter table {repositories_relation} add column if not exists policy_json text not null default '{{}}';",
        f"alter table {memberships_relation} add column if not exists repo_id text;",
        "\n".join(
            [
                f"update {memberships_relation} as m",
                "set repo_id = r.repo_id",
                f"from {repositories_relation} as r",
                "where m.repo_name = r.repo_name",
                "  and (m.repo_id is null or btrim(m.repo_id) = '');",
            ]
        ),
    ]
    preflight_queries = [
        "\n".join(
            [
                "select table_name, column_name, data_type",
                "from information_schema.columns",
                f"where table_schema = '{schema}'",
                f"  and table_name in ('{repositories_table}', '{memberships_table}')",
                "  and column_name in ('repo_id', 'repo_name', 'id_namespace_prefix', 'policy_json')",
                "order by table_name, column_name;",
            ]
        ),
        "\n".join(
            [
                "select id_namespace_prefix, array_agg(repo_name order by repo_name) as repo_names",
                f"from {repositories_relation}",
                "group by id_namespace_prefix",
                "having count(*) > 1;",
            ]
        ),
        "\n".join(
            [
                "select tablename, indexname, indexdef",
                "from pg_indexes",
                f"where schemaname = '{schema}'",
                f"  and tablename in ('{repositories_table}', '{memberships_table}')",
                "order by tablename, indexname;",
            ]
        ),
    ]
    index_sql = [
        f"create unique index if not exists {REPOSITORY_ID_UNIQUE_INDEX} on {repositories_relation}(repo_id);",
        f"create unique index if not exists {REPOSITORY_NAMESPACE_PREFIX_UNIQUE_INDEX} on {repositories_relation}(id_namespace_prefix);",
        f"create unique index if not exists {REPOSITORY_GROUP_MEMBERSHIPS_REPO_ID_UNIQUE_INDEX} on {memberships_relation}(repo_id);",
        f"create index if not exists {REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_ID_INDEX} on {memberships_relation}(group_id, sort_index, repo_id);",
        f"create index if not exists {REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_NAME_INDEX} on {memberships_relation}(group_id, sort_index, repo_name);",
    ]
    validation_queries = [
        f"select count(*) as null_repo_id_count from {repositories_relation} where repo_id is null;",
        f"select count(*) as null_namespace_prefix_count from {repositories_relation} where id_namespace_prefix is null or btrim(id_namespace_prefix) = '';",
        f"select count(*) as null_membership_repo_id_count from {memberships_relation} where repo_id is null or btrim(repo_id) = '';",
        "\n".join(
            [
                "select m.repo_name, m.repo_id as membership_repo_id, r.repo_id as repository_repo_id",
                f"from {memberships_relation} as m",
                f"join {repositories_relation} as r on r.repo_name = m.repo_name",
                "where m.repo_id is distinct from r.repo_id",
                "order by m.repo_name;",
            ]
        ),
        "\n".join(
            [
                "select indexname",
                "from pg_indexes",
                f"where schemaname = '{schema}'",
                f"  and tablename in ('{repositories_table}', '{memberships_table}')",
                "  and indexname in ('"
                + REPOSITORY_ID_UNIQUE_INDEX
                + "', '"
                + REPOSITORY_NAMESPACE_PREFIX_UNIQUE_INDEX
                + "', '"
                + REPOSITORY_GROUP_MEMBERSHIPS_REPO_ID_UNIQUE_INDEX
                + "', '"
                + REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_ID_INDEX
                + "', '"
                + REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_NAME_INDEX
                + "')",
                "order by indexname;",
            ]
        ),
    ]
    follow_up_notes = [
        "Run this helper only after the repositories-table rebuild artifact has been reviewed and the live cutover remains inside the frozen-write window.",
        "Pause if the namespace-prefix duplicate audit returns rows; the unique namespace index is only safe after collisions are resolved.",
        "Keep the legacy repository_group_memberships repo_name compatibility index until the dedicated repository_group_memberships rebuild slice lands.",
        "The repository_group_memberships slice still owns the final repo_name authority removal even if this helper backfills repo_id early for later waves.",
    ]
    return {
        "active_dsn_env": active_dsn_env,
        "repositories_relation": repositories_relation,
        "repository_group_memberships_relation": memberships_relation,
        "compatibility_column_sql": compatibility_column_sql,
        "preflight_queries": preflight_queries,
        "index_sql": index_sql,
        "validation_queries": validation_queries,
        "follow_up_notes": follow_up_notes,
    }


def format_plan(payload: dict[str, Any]) -> str:
    lines = [
        f"Active target: ${payload['active_dsn_env']}",
        f"Repositories relation: {payload['repositories_relation']}",
        f"Repository-group-memberships relation: {payload['repository_group_memberships_relation']}",
        "",
        "Preflight queries:",
    ]
    lines.extend(payload["preflight_queries"])
    lines.extend(["", "Compatibility-column SQL:"])
    lines.extend(payload["compatibility_column_sql"])
    lines.extend(["", "Index SQL:"])
    lines.extend(payload["index_sql"])
    lines.extend(["", "Validation queries:"])
    lines.extend(payload["validation_queries"])
    lines.extend(["", "Notes:"])
    for note in payload["follow_up_notes"]:
        lines.append(f"- {note}")
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--active-dsn-env", default=DEFAULT_ACTIVE_DSN_ENV, help=f"Environment variable name that holds the active database DSN. Default: {DEFAULT_ACTIVE_DSN_ENV}.")
    parser.add_argument("--schema", default=DEFAULT_SCHEMA, help=f"Target schema name. Default: {DEFAULT_SCHEMA}.")
    parser.add_argument("--repositories-table", default=DEFAULT_REPOSITORIES_TABLE, help=f"Repositories table name. Default: {DEFAULT_REPOSITORIES_TABLE}.")
    parser.add_argument("--memberships-table", default=DEFAULT_MEMBERSHIPS_TABLE, help=f"Repository-group-memberships table name. Default: {DEFAULT_MEMBERSHIPS_TABLE}.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of text.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    payload = build_foundation_indexes_plan(
        active_dsn_env=args.active_dsn_env,
        schema=args.schema,
        repositories_table=args.repositories_table,
        memberships_table=args.memberships_table,
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
