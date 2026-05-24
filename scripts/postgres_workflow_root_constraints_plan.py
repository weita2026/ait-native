#!/usr/bin/env python3
"""Render the Wave 3 workflow-root repo_id constraints/index plan."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

ROOT_CONSTRAINT_SPECS = [
    {
        "table": "plans",
        "statements": [
            "alter table public.plans alter column repo_id set not null;",
            "drop index if exists idx_plans_repo_updated;",
            "create index if not exists idx_plans_repo_id_updated on public.plans(repo_id, updated_at desc);",
        ],
    },
    {
        "table": "tasks",
        "statements": [
            "alter table public.tasks alter column repo_id set not null;",
            "drop index if exists idx_tasks_repo_created;",
            "create index if not exists idx_tasks_repo_id_created on public.tasks(repo_id, created_at desc);",
        ],
    },
    {
        "table": "changes",
        "statements": [
            "alter table public.changes alter column repo_id set not null;",
            "drop index if exists idx_changes_repo_updated;",
            "create index if not exists idx_changes_repo_id_updated on public.changes(repo_id, updated_at desc);",
        ],
    },
    {
        "table": "releases",
        "statements": [
            "alter table public.releases alter column repo_id set not null;",
            "alter table public.releases drop constraint if exists releases_repo_name_version_key;",
            "alter table public.releases add constraint releases_repo_id_version_key unique (repo_id, version);",
            "drop index if exists idx_releases_repo_updated;",
            "create index if not exists idx_releases_repo_id_updated on public.releases(repo_id, updated_at desc);",
        ],
    },
    {
        "table": "sessions",
        "statements": [
            "alter table public.sessions alter column repo_id set not null;",
            "drop index if exists idx_sessions_repo_updated;",
            "drop index if exists idx_sessions_repo_status;",
            "create index if not exists idx_sessions_repo_id_updated on public.sessions(repo_id, updated_at desc);",
            "create index if not exists idx_sessions_repo_id_status on public.sessions(repo_id, status, updated_at desc);",
        ],
    },
    {
        "table": "planning_sessions",
        "statements": [
            "alter table public.planning_sessions alter column repo_id set not null;",
            "drop index if exists idx_planning_sessions_repo_plan;",
            "create index if not exists idx_planning_sessions_repo_id_plan on public.planning_sessions(repo_id, plan_id, status, updated_at desc);",
        ],
    },
    {
        "table": "stacks",
        "statements": [
            "alter table public.stacks alter column repo_id set not null;",
            "drop index if exists idx_stacks_repo_updated;",
            "create index if not exists idx_stacks_repo_id_updated on public.stacks(repo_id, updated_at desc);",
        ],
    },
    {
        "table": "role_bindings",
        "statements": [
            "alter table public.role_bindings alter column repo_id set not null;",
            "alter table public.role_bindings drop constraint if exists role_bindings_repo_name_actor_identity_role_key;",
            "alter table public.role_bindings add constraint role_bindings_repo_id_actor_identity_role_key unique (repo_id, actor_identity, role);",
            "drop index if exists idx_role_bindings_repo_actor;",
            "create index if not exists idx_role_bindings_repo_id_actor on public.role_bindings(repo_id, actor_identity);",
        ],
    },
    {
        "table": "jobs",
        "statements": [
            "alter table public.jobs alter column repo_id set not null;",
            "drop index if exists idx_jobs_repo_state;",
            "create index if not exists idx_jobs_repo_id_state on public.jobs(repo_id, state, job_id);",
        ],
    },
    {
        "table": "authority_maps",
        "statements": [
            "alter table public.authority_maps alter column repo_id set not null;",
            "drop index if exists idx_authority_maps_repo;",
            "create unique index if not exists idx_authority_maps_repo_id_unique on public.authority_maps(repo_id);",
            "create index if not exists idx_authority_maps_repo_id_name on public.authority_maps(repo_id, repo_name);",
        ],
    },
]


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def build_workflow_root_constraints_plan(*, schema: str = "public") -> dict[str, Any]:
    steps: list[dict[str, Any]] = []
    for spec in ROOT_CONSTRAINT_SPECS:
        validation_queries = [
            f"select count(*) as null_repo_id_count from {schema}.{spec['table']} where repo_id is null or btrim(repo_id) = '';",
            "\n".join(
                [
                    "select indexname",
                    "from pg_indexes",
                    f"where schemaname = '{schema}'",
                    f"  and tablename = '{spec['table']}'",
                    "order by indexname;",
                ]
            ),
        ]
        steps.append({
            "table": spec["table"],
            "statements": spec["statements"],
            "validation_queries": validation_queries,
        })
    notes = [
        "Apply these statements only after the workflow-root rebuild helper reloads each table with repo_id backfilled.",
        "Keep any still-needed repo_name compatibility columns in place until downstream children and read models stop depending on them.",
        "Run the validation bundle after these statements land to compare row counts and repo-aware uniqueness expectations.",
    ]
    return {"schema": schema, "steps": steps, "notes": notes}


def format_plan(payload: dict[str, Any]) -> str:
    lines = [f"Schema: {payload['schema']}"]
    for step in payload["steps"]:
        lines.extend(["", f"[{step['table']}]", "Statements:"])
        lines.extend(step["statements"])
        lines.append("Validation queries:")
        lines.extend(step["validation_queries"])
    lines.extend(["", "Notes:"])
    lines.extend(f"- {note}" for note in payload["notes"])
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--schema", default="public", help="Schema name used in the rendered SQL. Default: public.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of text.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    payload = build_workflow_root_constraints_plan(schema=args.schema)
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
