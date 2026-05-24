#!/usr/bin/env python3
"""Render the Wave 1 PostgreSQL repository_group_memberships rebuild rehearsal plan."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

DEFAULT_ACTIVE_DSN_ENV = "AIT_NATIVE_SERVER_POSTGRES_DSN"
DEFAULT_ROLLBACK_DSN_ENV = "AIT_ROLLBACK_POSTGRES_DSN"
DEFAULT_EXPORT_PATH = "repository_group_memberships_wave1.csv"
REPOSITORY_GROUP_MEMBERSHIPS_REPO_ID_UNIQUE_INDEX = "idx_repository_group_memberships_repo_id_unique"
REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_ID_INDEX = "idx_repository_group_memberships_group_repo_id"
REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_NAME_INDEX = "idx_repository_group_memberships_group"
COLUMNS = ["repo_name", "repo_id", "group_id", "sort_index", "created_at", "updated_at"]


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _qualified_table(schema: str, table: str) -> str:
    return f'"{schema}"."{table}"'


def build_repository_group_memberships_rebuild_plan(
    *,
    rollback_dsn_env: str = DEFAULT_ROLLBACK_DSN_ENV,
    active_dsn_env: str = DEFAULT_ACTIVE_DSN_ENV,
    source_schema: str = "public",
    target_schema: str = "public",
    source_table: str = "repository_group_memberships",
    target_table: str = "repository_group_memberships",
    repositories_table: str = "repositories",
    repository_groups_table: str = "repository_groups",
    export_path: str = DEFAULT_EXPORT_PATH,
) -> dict[str, Any]:
    source_memberships_relation = _qualified_table(source_schema, source_table)
    source_repositories_relation = _qualified_table(source_schema, repositories_table)
    target_memberships_relation = _qualified_table(target_schema, target_table)
    target_repositories_relation = _qualified_table(target_schema, repositories_table)
    target_repository_groups_relation = _qualified_table(target_schema, repository_groups_table)
    column_list = ", ".join(COLUMNS)
    export_query = "\n".join(
        [
            f"select m.repo_name, coalesce(m.repo_id, r.repo_id) as repo_id, m.group_id, m.sort_index, m.created_at, m.updated_at",
            f"from {source_memberships_relation} as m",
            f"join {source_repositories_relation} as r on r.repo_name = m.repo_name",
            "order by m.group_id, m.sort_index, m.repo_name",
        ]
    )
    export_command = (
        f"psql \"${rollback_dsn_env}\" -v ON_ERROR_STOP=1 -c "
        f"\\\"\\copy ({export_query}) to '{export_path}' csv header\\\""
    )
    create_table_sql = "\n".join(
        [
            f"drop table if exists {target_memberships_relation} cascade;",
            "",
            f"create table {target_memberships_relation} (",
            f"    repo_id text primary key references {target_repositories_relation}(repo_id) on delete cascade,",
            f"    repo_name text not null unique references {target_repositories_relation}(repo_name) on delete cascade,",
            f"    group_id text not null references {target_repository_groups_relation}(group_id) on delete cascade,",
            "    sort_index integer not null,",
            "    created_at timestamptz not null,",
            "    updated_at timestamptz not null",
            ");",
        ]
    )
    post_create_index_sql = [
        f"create unique index if not exists {REPOSITORY_GROUP_MEMBERSHIPS_REPO_ID_UNIQUE_INDEX} on {target_memberships_relation}(repo_id);",
        f"create index if not exists {REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_ID_INDEX} on {target_memberships_relation}(group_id, sort_index, repo_id);",
        f"create index if not exists {REPOSITORY_GROUP_MEMBERSHIPS_GROUP_REPO_NAME_INDEX} on {target_memberships_relation}(group_id, sort_index, repo_name);",
    ]
    import_command = (
        f"psql \"${active_dsn_env}\" -v ON_ERROR_STOP=1 -c "
        f"\\\"\\copy {target_memberships_relation} ({column_list}) from '{export_path}' csv header\\\""
    )
    source_preflight_queries = [
        f"select count(*) as membership_count from {source_memberships_relation};",
        "\n".join(
            [
                "select m.repo_name",
                f"from {source_memberships_relation} as m",
                f"left join {source_repositories_relation} as r on r.repo_name = m.repo_name",
                "where r.repo_name is null",
                "order by m.repo_name;",
            ]
        ),
        "\n".join(
            [
                "select repo_name, count(*) as duplicates",
                f"from {source_memberships_relation}",
                "group by repo_name",
                "having count(*) > 1;",
            ]
        ),
    ]
    validation_queries = [
        f"select count(*) as membership_count from {target_memberships_relation};",
        f"select count(*) as null_repo_id_count from {target_memberships_relation} where repo_id is null or btrim(repo_id) = '';",
        f"select repo_name, count(*) as duplicates from {target_memberships_relation} group by repo_name having count(*) > 1;",
        f"select repo_id, count(*) as duplicates from {target_memberships_relation} group by repo_id having count(*) > 1;",
        "\n".join(
            [
                "select m.repo_name, m.repo_id as membership_repo_id, r.repo_id as repository_repo_id",
                f"from {target_memberships_relation} as m",
                f"join {target_repositories_relation} as r on r.repo_name = m.repo_name",
                "where m.repo_id is distinct from r.repo_id",
                "order by m.repo_name;",
            ]
        ),
        "\n".join(
            [
                "select m.group_id",
                f"from {target_memberships_relation} as m",
                f"left join {target_repository_groups_relation} as g on g.group_id = m.group_id",
                "where g.group_id is null",
                "order by m.group_id;",
            ]
        ),
    ]
    follow_up_notes = [
        "Run this helper after the repositories rebuild and foundation-indexes rehearsal both pass review inside the frozen-write window.",
        "Keep repo_id as the canonical membership authority; repo_name remains a compatibility and operator-readability column only.",
        "Retain the legacy group-by-repo_name compatibility index until readers no longer depend on repo_name ordering or lookup paths.",
        "Do not drop the repo_name column in this slice unless a later cleanup artifact proves all compatibility readers are gone.",
    ]
    return {
        "active_dsn_env": active_dsn_env,
        "rollback_dsn_env": rollback_dsn_env,
        "source_memberships_relation": source_memberships_relation,
        "source_repositories_relation": source_repositories_relation,
        "target_memberships_relation": target_memberships_relation,
        "target_repositories_relation": target_repositories_relation,
        "target_repository_groups_relation": target_repository_groups_relation,
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
        f"Rollback source: ${payload['rollback_dsn_env']} -> {payload['source_memberships_relation']}",
        f"Active target: ${payload['active_dsn_env']} -> {payload['target_memberships_relation']}",
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
    parser.add_argument("--source-table", default="repository_group_memberships", help="Rollback/source memberships table name. Default: repository_group_memberships.")
    parser.add_argument("--target-table", default="repository_group_memberships", help="Active target memberships table name. Default: repository_group_memberships.")
    parser.add_argument("--repositories-table", default="repositories", help="Repositories table name in both source and target schemas. Default: repositories.")
    parser.add_argument("--repository-groups-table", default="repository_groups", help="Repository-groups table name in the target schema. Default: repository_groups.")
    parser.add_argument("--export-path", default=DEFAULT_EXPORT_PATH, help=f"CSV export path used in the rendered commands. Default: {DEFAULT_EXPORT_PATH}.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of text.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    payload = build_repository_group_memberships_rebuild_plan(
        rollback_dsn_env=args.rollback_dsn_env,
        active_dsn_env=args.active_dsn_env,
        source_schema=args.source_schema,
        target_schema=args.target_schema,
        source_table=args.source_table,
        target_table=args.target_table,
        repositories_table=args.repositories_table,
        repository_groups_table=args.repository_groups_table,
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
