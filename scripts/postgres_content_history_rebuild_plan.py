#!/usr/bin/env python3
"""Render the Wave 2 PostgreSQL content-history rebuild rehearsal plan."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

DEFAULT_ACTIVE_DSN_ENV = "AIT_NATIVE_SERVER_POSTGRES_DSN"
DEFAULT_ROLLBACK_DSN_ENV = "AIT_ROLLBACK_POSTGRES_DSN"
DEFAULT_SNAPSHOTS_EXPORT_PATH = "snapshots_wave2.csv"
DEFAULT_PACKS_EXPORT_PATH = "packs_wave2.csv"
SNAPSHOTS_REPO_COMPAT_INDEX = "idx_snapshots_repo_created"
SNAPSHOTS_REPO_ID_CREATED_INDEX = "idx_snapshots_repo_id_created"
PACKS_REPO_COMPAT_INDEX = "idx_packs_repo"
PACKS_REPO_ID_INDEX = "idx_packs_repo_id"
SNAPSHOTS_COLUMNS = [
    "snapshot_id",
    "repo_name",
    "repo_id",
    "parent_snapshot_id",
    "root_tree_id",
    "manifest_hash",
    "manifest_path",
    "message",
    "line_name",
    "file_count",
    "total_bytes",
    "created_at",
]
PACKS_COLUMNS = [
    "pack_id",
    "repo_name",
    "repo_id",
    "status",
    "member_count",
    "total_bytes",
    "pack_path",
    "pack_format",
    "pack_index_entry_name",
    "pack_index_checksum",
    "created_at",
]


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _qualified_table(schema: str, table: str) -> str:
    return f'"{schema}"."{table}"'


def _table_plan(
    *,
    rollback_dsn_env: str,
    active_dsn_env: str,
    source_relation: str,
    target_relation: str,
    source_repositories_relation: str,
    target_repositories_relation: str,
    target_lines_relation: str,
    export_query: str,
    export_path: str,
    columns: list[str],
    create_table_sql: str,
    post_create_index_sql: list[str],
    source_preflight_queries: list[str],
    validation_queries: list[str],
) -> dict[str, Any]:
    column_list = ", ".join(columns)
    export_command = (
        f'psql "${rollback_dsn_env}" -v ON_ERROR_STOP=1 -c '
        f'\\"\\copy ({export_query}) to \'{export_path}\' csv header\\"'
    )
    import_command = (
        f'psql "${active_dsn_env}" -v ON_ERROR_STOP=1 -c '
        f'\\"\\copy {target_relation} ({column_list}) from \'{export_path}\' csv header\\"'
    )
    return {
        "source_relation": source_relation,
        "target_relation": target_relation,
        "source_repositories_relation": source_repositories_relation,
        "target_repositories_relation": target_repositories_relation,
        "target_lines_relation": target_lines_relation,
        "export_path": export_path,
        "columns": columns,
        "source_preflight_queries": source_preflight_queries,
        "export_query": export_query,
        "export_command": export_command,
        "create_table_sql": create_table_sql,
        "post_create_index_sql": post_create_index_sql,
        "import_command": import_command,
        "validation_queries": validation_queries,
    }


def build_content_history_rebuild_plan(
    *,
    rollback_dsn_env: str = DEFAULT_ROLLBACK_DSN_ENV,
    active_dsn_env: str = DEFAULT_ACTIVE_DSN_ENV,
    source_schema: str = "public",
    target_schema: str = "public",
    snapshots_table: str = "snapshots",
    packs_table: str = "packs",
    repositories_table: str = "repositories",
    lines_table: str = "lines",
    snapshots_export_path: str = DEFAULT_SNAPSHOTS_EXPORT_PATH,
    packs_export_path: str = DEFAULT_PACKS_EXPORT_PATH,
) -> dict[str, Any]:
    source_snapshots_relation = _qualified_table(source_schema, snapshots_table)
    source_packs_relation = _qualified_table(source_schema, packs_table)
    source_repositories_relation = _qualified_table(source_schema, repositories_table)
    target_snapshots_relation = _qualified_table(target_schema, snapshots_table)
    target_packs_relation = _qualified_table(target_schema, packs_table)
    target_repositories_relation = _qualified_table(target_schema, repositories_table)
    target_lines_relation = _qualified_table(target_schema, lines_table)

    snapshots_export_query = "\n".join(
        [
            "select s.snapshot_id, s.repo_name, coalesce(s.repo_id, r.repo_id) as repo_id, s.parent_snapshot_id, s.root_tree_id, s.manifest_hash, s.manifest_path, s.message, s.line_name, s.file_count, s.total_bytes, s.created_at",
            f"from {source_snapshots_relation} as s",
            f"join {source_repositories_relation} as r on r.repo_name = s.repo_name",
            "order by s.created_at, s.snapshot_id",
        ]
    )
    snapshots_create_table_sql = "\n".join(
        [
            f"drop table if exists {target_snapshots_relation} cascade;",
            "",
            f"create table {target_snapshots_relation} (",
            "    snapshot_id text primary key,",
            f"    repo_name text not null references {target_repositories_relation}(repo_name) on delete cascade,",
            f"    repo_id text not null references {target_repositories_relation}(repo_id) on delete cascade,",
            f"    parent_snapshot_id text references {target_snapshots_relation}(snapshot_id) on delete set null,",
            "    root_tree_id text,",
            "    manifest_hash text not null default '',",
            "    manifest_path text not null default '',",
            "    message text,",
            "    line_name text,",
            "    file_count integer not null,",
            "    total_bytes integer not null,",
            "    created_at timestamptz not null",
            ");",
        ]
    )
    snapshots_post_create_index_sql = [
        f"create index if not exists {SNAPSHOTS_REPO_COMPAT_INDEX} on {target_snapshots_relation}(repo_name, created_at desc);",
        f"create index if not exists {SNAPSHOTS_REPO_ID_CREATED_INDEX} on {target_snapshots_relation}(repo_id, created_at desc);",
    ]
    snapshots_source_preflight_queries = [
        f"select count(*) as snapshot_count from {source_snapshots_relation};",
        "\n".join(
            [
                "select s.snapshot_id, s.repo_name",
                f"from {source_snapshots_relation} as s",
                f"left join {source_repositories_relation} as r on r.repo_name = s.repo_name",
                "where r.repo_name is null",
                "order by s.created_at, s.snapshot_id;",
            ]
        ),
        "\n".join(
            [
                "select s.snapshot_id, s.repo_name, s.line_name",
                f"from {source_snapshots_relation} as s",
                "where s.line_name is not null and (s.repo_id is null or btrim(s.repo_id) = '')",
                "order by s.created_at, s.snapshot_id;",
            ]
        ),
    ]
    snapshots_validation_queries = [
        f"select count(*) as snapshot_count from {target_snapshots_relation};",
        f"select count(*) as null_repo_id_count from {target_snapshots_relation} where repo_id is null or btrim(repo_id) = '';",
        "\n".join(
            [
                "select s.repo_name, s.repo_id as snapshot_repo_id, r.repo_id as repository_repo_id",
                f"from {target_snapshots_relation} as s",
                f"join {target_repositories_relation} as r on r.repo_name = s.repo_name",
                "where s.repo_id is distinct from r.repo_id",
                "order by s.created_at, s.snapshot_id;",
            ]
        ),
        "\n".join(
            [
                "select s.snapshot_id, s.repo_id, s.line_name",
                f"from {target_snapshots_relation} as s",
                f"left join {target_lines_relation} as l on l.repo_id = s.repo_id and l.line_name = s.line_name",
                "where s.line_name is not null and l.line_name is null",
                "order by s.created_at, s.snapshot_id;",
            ]
        ),
        "\n".join(
            [
                "select s.snapshot_id, s.parent_snapshot_id",
                f"from {target_snapshots_relation} as s",
                f"left join {target_snapshots_relation} as parent on parent.snapshot_id = s.parent_snapshot_id",
                "where s.parent_snapshot_id is not null and parent.snapshot_id is null",
                "order by s.created_at, s.snapshot_id;",
            ]
        ),
        "\n".join(
            [
                "select indexname",
                "from pg_indexes",
                f"where schemaname = '{target_schema}'",
                f"  and tablename = '{snapshots_table}'",
                f"  and indexname in ('{SNAPSHOTS_REPO_COMPAT_INDEX}', '{SNAPSHOTS_REPO_ID_CREATED_INDEX}')",
                "order by indexname;",
            ]
        ),
    ]

    packs_export_query = "\n".join(
        [
            "select p.pack_id, p.repo_name, coalesce(p.repo_id, r.repo_id) as repo_id, p.status, p.member_count, p.total_bytes, p.pack_path, p.pack_format, p.pack_index_entry_name, p.pack_index_checksum, p.created_at",
            f"from {source_packs_relation} as p",
            f"join {source_repositories_relation} as r on r.repo_name = p.repo_name",
            "order by p.created_at, p.pack_id",
        ]
    )
    packs_create_table_sql = "\n".join(
        [
            f"drop table if exists {target_packs_relation} cascade;",
            "",
            f"create table {target_packs_relation} (",
            "    pack_id text primary key,",
            f"    repo_name text not null references {target_repositories_relation}(repo_name) on delete cascade,",
            f"    repo_id text not null references {target_repositories_relation}(repo_id) on delete cascade,",
            "    status text not null,",
            "    member_count integer not null,",
            "    total_bytes integer not null,",
            "    pack_path text,",
            "    pack_format text not null default 'ait-pack-v1',",
            "    pack_index_entry_name text,",
            "    pack_index_checksum text,",
            "    created_at timestamptz not null",
            ");",
        ]
    )
    packs_post_create_index_sql = [
        f"create index if not exists {PACKS_REPO_COMPAT_INDEX} on {target_packs_relation}(repo_name, created_at desc);",
        f"create index if not exists {PACKS_REPO_ID_INDEX} on {target_packs_relation}(repo_id, created_at desc);",
    ]
    packs_source_preflight_queries = [
        f"select count(*) as pack_count from {source_packs_relation};",
        "\n".join(
            [
                "select p.pack_id, p.repo_name",
                f"from {source_packs_relation} as p",
                f"left join {source_repositories_relation} as r on r.repo_name = p.repo_name",
                "where r.repo_name is null",
                "order by p.created_at, p.pack_id;",
            ]
        ),
        "\n".join(
            [
                "select p.pack_id, p.repo_name",
                f"from {source_packs_relation} as p",
                "where p.repo_id is null or btrim(p.repo_id) = ''",
                "order by p.created_at, p.pack_id;",
            ]
        ),
    ]
    packs_validation_queries = [
        f"select count(*) as pack_count from {target_packs_relation};",
        f"select count(*) as null_repo_id_count from {target_packs_relation} where repo_id is null or btrim(repo_id) = '';",
        "\n".join(
            [
                "select p.repo_name, p.repo_id as pack_repo_id, r.repo_id as repository_repo_id",
                f"from {target_packs_relation} as p",
                f"join {target_repositories_relation} as r on r.repo_name = p.repo_name",
                "where p.repo_id is distinct from r.repo_id",
                "order by p.created_at, p.pack_id;",
            ]
        ),
        "\n".join(
            [
                "select p.pack_id, p.pack_path",
                f"from {target_packs_relation} as p",
                "where p.pack_path is not null and btrim(p.pack_path) = ''",
                "order by p.created_at, p.pack_id;",
            ]
        ),
        "\n".join(
            [
                "select indexname",
                "from pg_indexes",
                f"where schemaname = '{target_schema}'",
                f"  and tablename = '{packs_table}'",
                f"  and indexname in ('{PACKS_REPO_COMPAT_INDEX}', '{PACKS_REPO_ID_INDEX}')",
                "order by indexname;",
            ]
        ),
    ]

    follow_up_notes = [
        "Run this helper after the Wave 2 lines slice lands so snapshot line_name checks can validate canonical repo_id + line_name lineage.",
        "Keep repo_id as the canonical content-history authority, but retain repo_name temporarily for operator readability and legacy lookup compatibility during the frozen-write window.",
        "Leave repo-neutral content tables such as blobs, trees, tree_entries, and tree_packs in place unless the separate content-exceptions audit proves they also need repair.",
        "Do not remove the repo_name compatibility indexes until downstream readers and rollout checks no longer depend on repo_name-scoped history lookup.",
    ]

    return {
        "active_dsn_env": active_dsn_env,
        "rollback_dsn_env": rollback_dsn_env,
        "snapshots": _table_plan(
            rollback_dsn_env=rollback_dsn_env,
            active_dsn_env=active_dsn_env,
            source_relation=source_snapshots_relation,
            target_relation=target_snapshots_relation,
            source_repositories_relation=source_repositories_relation,
            target_repositories_relation=target_repositories_relation,
            target_lines_relation=target_lines_relation,
            export_query=snapshots_export_query,
            export_path=snapshots_export_path,
            columns=SNAPSHOTS_COLUMNS,
            create_table_sql=snapshots_create_table_sql,
            post_create_index_sql=snapshots_post_create_index_sql,
            source_preflight_queries=snapshots_source_preflight_queries,
            validation_queries=snapshots_validation_queries,
        ),
        "packs": _table_plan(
            rollback_dsn_env=rollback_dsn_env,
            active_dsn_env=active_dsn_env,
            source_relation=source_packs_relation,
            target_relation=target_packs_relation,
            source_repositories_relation=source_repositories_relation,
            target_repositories_relation=target_repositories_relation,
            target_lines_relation=target_lines_relation,
            export_query=packs_export_query,
            export_path=packs_export_path,
            columns=PACKS_COLUMNS,
            create_table_sql=packs_create_table_sql,
            post_create_index_sql=packs_post_create_index_sql,
            source_preflight_queries=packs_source_preflight_queries,
            validation_queries=packs_validation_queries,
        ),
        "follow_up_notes": follow_up_notes,
    }


def format_plan(payload: dict[str, Any]) -> str:
    lines = [
        f"Rollback source DSN env: ${payload['rollback_dsn_env']}",
        f"Active target DSN env: ${payload['active_dsn_env']}",
    ]
    for label in ("snapshots", "packs"):
        section = payload[label]
        lines.extend(
            [
                "",
                f"[{label}]",
                f"Rollback source: {section['source_relation']}",
                f"Active target: {section['target_relation']}",
                f"Export path: {section['export_path']}",
                "",
                "Source preflight queries:",
            ]
        )
        lines.extend(section["source_preflight_queries"])
        lines.extend([
            "",
            "Rollback export command:",
            section["export_command"],
            "",
            "Active rebuild DDL:",
            section["create_table_sql"],
            "",
            "Post-create index SQL:",
        ])
        lines.extend(section["post_create_index_sql"])
        lines.extend([
            "",
            "Active import command:",
            section["import_command"],
            "",
            "Validation queries:",
        ])
        lines.extend(section["validation_queries"])
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
    parser.add_argument("--snapshots-table", default="snapshots", help="Snapshots table name in both source and target schemas. Default: snapshots.")
    parser.add_argument("--packs-table", default="packs", help="Packs table name in both source and target schemas. Default: packs.")
    parser.add_argument("--repositories-table", default="repositories", help="Repositories table name in both source and target schemas. Default: repositories.")
    parser.add_argument("--lines-table", default="lines", help="Lines table name in the active target schema. Default: lines.")
    parser.add_argument("--snapshots-export-path", default=DEFAULT_SNAPSHOTS_EXPORT_PATH, help=f"CSV export path used for snapshots commands. Default: {DEFAULT_SNAPSHOTS_EXPORT_PATH}.")
    parser.add_argument("--packs-export-path", default=DEFAULT_PACKS_EXPORT_PATH, help=f"CSV export path used for packs commands. Default: {DEFAULT_PACKS_EXPORT_PATH}.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of text.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    payload = build_content_history_rebuild_plan(
        rollback_dsn_env=args.rollback_dsn_env,
        active_dsn_env=args.active_dsn_env,
        source_schema=args.source_schema,
        target_schema=args.target_schema,
        snapshots_table=args.snapshots_table,
        packs_table=args.packs_table,
        repositories_table=args.repositories_table,
        lines_table=args.lines_table,
        snapshots_export_path=args.snapshots_export_path,
        packs_export_path=args.packs_export_path,
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
