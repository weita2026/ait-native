#!/usr/bin/env python3
"""Render the Wave 5 row-count and uniqueness validation plan."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

DEFAULT_SCHEMA = "public"
VALIDATION_TABLES = [
    "repositories",
    "repository_group_memberships",
    "lines",
    "snapshots",
    "packs",
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
]


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def build_repo_id_row_count_validation(*, schema: str = DEFAULT_SCHEMA) -> dict[str, Any]:
    tables: list[dict[str, Any]] = []
    for table in VALIDATION_TABLES:
        entry = {
            "table": table,
            "rollback_count_query": f"select count(*) as row_count from {schema}.{table};",
            "active_count_query": f"select count(*) as row_count from {schema}.{table};",
            "repo_id_null_query": f"select count(*) as null_repo_id_count from {schema}.{table} where repo_id is null or btrim(repo_id) = '';" if table not in {"repositories"} else f"select count(*) as null_repo_id_count from {schema}.repositories where repo_id is null or btrim(repo_id) = '';",
            "notes": ["Compare rollback vs active counts after every rebuild wave completes."] if table in {"repositories", "lines", "snapshots", "packs", "plans", "tasks", "changes"} else ["Use this count check after the parent/root validation is already green."],
        }
        uniqueness_queries = []
        if table == "repositories":
            uniqueness_queries.extend([
                f"select repo_id, count(*) as duplicates from {schema}.repositories group by repo_id having count(*) > 1;",
                f"select repo_name, count(*) as duplicates from {schema}.repositories group by repo_name having count(*) > 1;",
            ])
        elif table == "repository_group_memberships":
            uniqueness_queries.append(f"select repo_id, count(*) as duplicates from {schema}.repository_group_memberships group by repo_id having count(*) > 1;")
        elif table == "lines":
            uniqueness_queries.append(f"select repo_id, line_name, count(*) as duplicates from {schema}.lines group by repo_id, line_name having count(*) > 1;")
        elif table == "releases":
            uniqueness_queries.append(f"select repo_id, version, count(*) as duplicates from {schema}.releases group by repo_id, version having count(*) > 1;")
        elif table == "role_bindings":
            uniqueness_queries.append(f"select repo_id, actor_identity, role, count(*) as duplicates from {schema}.role_bindings group by repo_id, actor_identity, role having count(*) > 1;")
        entry["uniqueness_queries"] = uniqueness_queries
        tables.append(entry)
    notes = [
        "Run the rollback-count query against the frozen rollback source and compare it with the active query output after each rebuild wave.",
        "A zero-difference row count is necessary but not sufficient; keep the uniqueness and repo_id-null checks in the same validation packet.",
        "Do not advance to rollback cleanup until every rebuilt table in this bundle reports equal counts and zero repo_id-null rows where repo_id is expected.",
    ]
    return {"schema": schema, "tables": tables, "notes": notes}


def format_plan(payload: dict[str, Any]) -> str:
    lines = [f"Schema: {payload['schema']}"]
    for entry in payload["tables"]:
        lines.extend(["", f"[{entry['table']}]", "Rollback count query:", entry["rollback_count_query"], "Active count query:", entry["active_count_query"], "Repo-id null query:", entry["repo_id_null_query"]])
        if entry["uniqueness_queries"]:
            lines.append("Uniqueness queries:")
            lines.extend(entry["uniqueness_queries"])
        lines.append("Notes:")
        lines.extend(f"- {note}" for note in entry["notes"])
    lines.extend(["", "Notes:"])
    lines.extend(f"- {note}" for note in payload["notes"])
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--schema", default=DEFAULT_SCHEMA, help="Schema name used in the rendered SQL. Default: public.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of text.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    payload = build_repo_id_row_count_validation(schema=args.schema)
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
