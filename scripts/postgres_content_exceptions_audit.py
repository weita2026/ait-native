#!/usr/bin/env python3
"""Render the Wave 2 repo-neutral content exceptions audit plan."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

DEFAULT_ACTIVE_DSN_ENV = "AIT_NATIVE_SERVER_POSTGRES_DSN"
DEFAULT_ROLLBACK_DSN_ENV = "AIT_ROLLBACK_POSTGRES_DSN"
REPO_NEUTRAL_TABLES = ["blobs", "trees", "tree_entries", "tree_packs"]


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _qualified_table(schema: str, table: str) -> str:
    return f'"{schema}"."{table}"'


def build_content_exceptions_audit(
    *,
    rollback_dsn_env: str = DEFAULT_ROLLBACK_DSN_ENV,
    active_dsn_env: str = DEFAULT_ACTIVE_DSN_ENV,
    schema: str = "public",
    blobs_table: str = "blobs",
    trees_table: str = "trees",
    tree_entries_table: str = "tree_entries",
    tree_packs_table: str = "tree_packs",
    snapshots_table: str = "snapshots",
    packs_table: str = "packs",
) -> dict[str, Any]:
    blobs_relation = _qualified_table(schema, blobs_table)
    trees_relation = _qualified_table(schema, trees_table)
    tree_entries_relation = _qualified_table(schema, tree_entries_table)
    tree_packs_relation = _qualified_table(schema, tree_packs_table)
    snapshots_relation = _qualified_table(schema, snapshots_table)
    packs_relation = _qualified_table(schema, packs_table)
    table_roles = {
        "blobs": "blob_id/sha256 content-addressed storage; no repo_name authority column",
        "trees": "tree_id-addressed tree objects; may reference tree_packs but not repositories directly",
        "tree_entries": "tree-local entry graph; target_id points at blobs or trees rather than repositories",
        "tree_packs": "tree-pack archives keyed by pack_id; repo scope arrives only through snapshots/tree references",
    }
    rollback_audit_queries = [
        f"select count(*) as blob_count from {blobs_relation};",
        f"select count(*) as tree_count from {trees_relation};",
        f"select count(*) as tree_entry_count from {tree_entries_relation};",
        f"select count(*) as tree_pack_count from {tree_packs_relation};",
    ]
    active_dependency_queries = [
        "\n".join(
            [
                "select s.snapshot_id, s.root_tree_id",
                f"from {snapshots_relation} as s",
                f"left join {trees_relation} as t on t.tree_id = s.root_tree_id",
                "where s.root_tree_id is not null and t.tree_id is null",
                "order by s.created_at, s.snapshot_id;",
            ]
        ),
        "\n".join(
            [
                "select t.tree_id, t.tree_pack_id",
                f"from {trees_relation} as t",
                f"left join {tree_packs_relation} as tp on tp.pack_id = t.tree_pack_id",
                "where t.tree_pack_id is not null and tp.pack_id is null",
                "order by t.tree_id;",
            ]
        ),
        "\n".join(
            [
                "select b.blob_id, b.pack_id",
                f"from {blobs_relation} as b",
                f"left join {packs_relation} as p on p.pack_id = b.pack_id",
                "where b.pack_id is not null and p.pack_id is null",
                "order by b.blob_id;",
            ]
        ),
        "\n".join(
            [
                "select te.tree_id, te.entry_name, te.target_id, te.entry_type",
                f"from {tree_entries_relation} as te",
                f"left join {blobs_relation} as b on b.blob_id = te.target_id and te.entry_type = 'blob'",
                f"left join {trees_relation} as t on t.tree_id = te.target_id and te.entry_type = 'tree'",
                "where (te.entry_type = 'blob' and b.blob_id is null)",
                "   or (te.entry_type = 'tree' and t.tree_id is null)",
                "order by te.tree_id, te.entry_name;",
            ]
        ),
    ]
    decision_notes = [
        "These tables stay in place because their keys are content IDs rather than repository identifiers.",
        "Only rebuild them if the dependency audit above shows orphaned lineage caused by the snapshots or packs rebuild waves.",
        "Keep the rollback database available until the repo-scoped smoke checks confirm content-history reads still resolve trees, blobs, and pack archives correctly.",
    ]
    return {
        "active_dsn_env": active_dsn_env,
        "rollback_dsn_env": rollback_dsn_env,
        "schema": schema,
        "repo_neutral_tables": REPO_NEUTRAL_TABLES,
        "table_roles": table_roles,
        "rollback_audit_queries": rollback_audit_queries,
        "active_dependency_queries": active_dependency_queries,
        "decision_notes": decision_notes,
    }


def format_audit(payload: dict[str, Any]) -> str:
    lines = [
        f"Rollback source DSN env: ${payload['rollback_dsn_env']}",
        f"Active target DSN env: ${payload['active_dsn_env']}",
        f"Schema: {payload['schema']}",
        "",
        "Repo-neutral content tables:",
    ]
    for table in payload["repo_neutral_tables"]:
        lines.append(f"- {table}: {payload['table_roles'][table]}")
    lines.extend(["", "Rollback inventory queries:"])
    lines.extend(payload["rollback_audit_queries"])
    lines.extend(["", "Active dependency audit queries:"])
    lines.extend(payload["active_dependency_queries"])
    lines.extend(["", "Decision notes:"])
    for note in payload["decision_notes"]:
        lines.append(f"- {note}")
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rollback-dsn-env", default=DEFAULT_ROLLBACK_DSN_ENV, help=f"Environment variable name that holds the rollback database DSN. Default: {DEFAULT_ROLLBACK_DSN_ENV}.")
    parser.add_argument("--active-dsn-env", default=DEFAULT_ACTIVE_DSN_ENV, help=f"Environment variable name that holds the active database DSN. Default: {DEFAULT_ACTIVE_DSN_ENV}.")
    parser.add_argument("--schema", default="public", help="Schema name used for all rendered relations. Default: public.")
    parser.add_argument("--blobs-table", default="blobs", help="Blobs table name. Default: blobs.")
    parser.add_argument("--trees-table", default="trees", help="Trees table name. Default: trees.")
    parser.add_argument("--tree-entries-table", default="tree_entries", help="Tree entries table name. Default: tree_entries.")
    parser.add_argument("--tree-packs-table", default="tree_packs", help="Tree packs table name. Default: tree_packs.")
    parser.add_argument("--snapshots-table", default="snapshots", help="Snapshots table name. Default: snapshots.")
    parser.add_argument("--packs-table", default="packs", help="Packs table name. Default: packs.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of text.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    payload = build_content_exceptions_audit(
        rollback_dsn_env=args.rollback_dsn_env,
        active_dsn_env=args.active_dsn_env,
        schema=args.schema,
        blobs_table=args.blobs_table,
        trees_table=args.trees_table,
        tree_entries_table=args.tree_entries_table,
        tree_packs_table=args.tree_packs_table,
        snapshots_table=args.snapshots_table,
        packs_table=args.packs_table,
    )
    text = _json_dump(payload) if args.json else format_audit(payload)
    if args.output:
        output_path = Path(args.output).expanduser()
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(text + ("" if text.endswith("\n") else "\n"), encoding="utf-8")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
