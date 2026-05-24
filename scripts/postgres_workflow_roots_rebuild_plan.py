#!/usr/bin/env python3
"""Render the Wave 3 workflow-root rebuild rehearsal plan."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

DEFAULT_ACTIVE_DSN_ENV = "AIT_NATIVE_SERVER_POSTGRES_DSN"
DEFAULT_ROLLBACK_DSN_ENV = "AIT_ROLLBACK_POSTGRES_DSN"
DEFAULT_EXPORT_DIR = "workflow_roots_wave3"
ROOT_TABLE_SPECS = [
    {
        "table": "plans",
        "primary_key": "plan_id",
        "order_by": "updated_at desc, plan_id",
        "columns": ["plan_id", "repo_name", "repo_id", "title", "status", "head_revision_id", "created_by", "created_at", "updated_at"],
        "repo_name_unique": False,
        "notes": ["Preserve plan head linkage while forcing repo_id not null.", "Let the root-constraints helper replace repo_name-scoped indexes with repo_id-aware equivalents after reload."],
    },
    {
        "table": "tasks",
        "primary_key": "task_id",
        "order_by": "created_at desc, task_id",
        "columns": ["task_id", "repo_name", "repo_id", "task_seq", "title", "intent", "risk_tier", "planning_state", "plan_id", "origin_plan_revision_id", "plan_item_ref", "plan_section_ref", "plan_drift_state", "plan_linked_at", "source_completion_mode", "source_local_task_id", "source_local_completed_at", "historical_publication_id", "status", "created_at"],
        "repo_name_unique": False,
        "notes": ["Keep existing task identity columns and plan lineage fields unchanged.", "Backfill repo_id from repositories before any downstream child tables are rebuilt."],
    },
    {
        "table": "changes",
        "primary_key": "change_id",
        "order_by": "updated_at desc, change_id",
        "columns": ["change_id", "repo_name", "repo_id", "change_seq", "task_id", "title", "base_line", "fork_snapshot_id", "forked_from_line", "risk_tier", "lane", "source_completion_mode", "source_local_change_id", "source_local_status", "source_target_line", "source_landed_snapshot_id", "source_landed_at", "historical_publication_id", "status", "current_patchset_number", "selected_patchset_number", "created_at", "updated_at", "landed_at"],
        "repo_name_unique": False,
        "notes": ["Do not change the task/change link contract in this slice.", "Keep patchset numbering untouched while repo scope becomes canonical."],
    },
    {
        "table": "releases",
        "primary_key": "release_id",
        "order_by": "updated_at desc, release_id",
        "columns": ["release_id", "repo_name", "repo_id", "version", "line_name", "snapshot_id", "manifest_hash", "profile", "package_name", "package_version", "package_requires_python", "status", "checks_json", "artifacts_json", "formula_json", "metadata_json", "created_by", "actor_type", "created_at", "updated_at"],
        "repo_name_unique": False,
        "notes": ["Preserve release identity and version history while shifting uniqueness later to repo_id + version.", "Do not rewrite release payload JSON in this root-table slice."],
    },
    {
        "table": "sessions",
        "primary_key": "session_id",
        "order_by": "updated_at desc, session_id",
        "columns": ["session_id", "repo_name", "repo_id", "session_local_id", "task_id", "change_id", "title", "session_kind", "status", "line_name", "worktree_name", "model_name", "actor_identity", "actor_type", "metadata_json", "last_event_sequence", "head_checkpoint_id", "created_at", "updated_at"],
        "repo_name_unique": False,
        "notes": ["Retain task/change bindings and checkpoint references.", "Queue/read-model payload lineage gets handled in the later payload-lineage slice."],
    },
    {
        "table": "planning_sessions",
        "primary_key": "planning_session_id",
        "order_by": "updated_at desc, planning_session_id",
        "columns": ["planning_session_id", "repo_name", "repo_id", "planning_session_local_id", "plan_id", "title", "mode", "status", "preferred_agent", "artifact_status", "derived_task_id", "last_promoted_plan_revision_id", "last_event_sequence", "created_by", "created_at", "updated_at"],
        "repo_name_unique": False,
        "notes": ["Keep planning-session lineage stable for derived tasks and promoted revisions.", "The child events tables are rebuilt later after this root slice lands."],
    },
    {
        "table": "stacks",
        "primary_key": "stack_id",
        "order_by": "updated_at desc, stack_id",
        "columns": ["stack_id", "repo_name", "repo_id", "stack_seq", "title", "landing_policy", "status", "created_at", "updated_at"],
        "repo_name_unique": False,
        "notes": ["Preserve stack identity and landing policy semantics.", "Stack membership rows move in the workflow-children slice."],
    },
    {
        "table": "role_bindings",
        "primary_key": "binding_id",
        "order_by": "created_at desc, binding_id",
        "columns": ["binding_id", "repo_name", "repo_id", "actor_identity", "role", "created_at"],
        "repo_name_unique": False,
        "notes": ["Rebuild role bindings before repo_id-aware uniqueness is tightened.", "Keep actor_identity/role values untouched during the copy."],
    },
    {
        "table": "jobs",
        "primary_key": "job_id",
        "order_by": "created_at desc, job_id",
        "columns": ["job_id", "repo_name", "repo_id", "job_type", "state", "payload_json", "result_json", "attempt_count", "max_attempts", "available_at", "locked_at", "locked_by", "last_error", "created_at", "updated_at"],
        "repo_name_unique": False,
        "notes": ["Do not rewrite payload_json in this root rebuild; only canonicalize the top-level repo scope column.", "Payload lineage audits happen in the later payload-lineage slice."],
    },
    {
        "table": "authority_maps",
        "primary_key": "authority_map_id",
        "order_by": "updated_at desc, authority_map_id",
        "columns": ["authority_map_id", "repo_name", "repo_id", "root_document_path", "milestone_document_path", "schema_version", "created_at", "updated_at"],
        "repo_name_unique": True,
        "notes": ["Preserve document-path authority routing while repo_id becomes mandatory.", "Repo-name uniqueness can remain temporarily until downstream surfaces switch fully to repo_id-aware routing."],
    },
]


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _qualified_table(schema: str, table: str) -> str:
    return f'"{schema}"."{table}"'


def build_workflow_roots_rebuild_plan(
    *,
    rollback_dsn_env: str = DEFAULT_ROLLBACK_DSN_ENV,
    active_dsn_env: str = DEFAULT_ACTIVE_DSN_ENV,
    source_schema: str = "public",
    target_schema: str = "public",
    repositories_table: str = "repositories",
    export_dir: str = DEFAULT_EXPORT_DIR,
) -> dict[str, Any]:
    source_repositories_relation = _qualified_table(source_schema, repositories_table)
    target_repositories_relation = _qualified_table(target_schema, repositories_table)
    table_plans: list[dict[str, Any]] = []
    for spec in ROOT_TABLE_SPECS:
        source_relation = _qualified_table(source_schema, spec["table"])
        target_relation = _qualified_table(target_schema, spec["table"])
        export_path = f"{export_dir}/{spec['table']}.csv"
        columns = list(spec["columns"])
        projection_columns = [
            "coalesce(t.repo_id, r.repo_id) as repo_id" if column == "repo_id" else f"t.{column}"
            for column in columns
        ]
        export_query = "\n".join(
            [
                f"select {', '.join(projection_columns)}",
                f"from {source_relation} as t",
                f"join {source_repositories_relation} as r on r.repo_name = t.repo_name",
                f"order by {spec['order_by']}",
            ]
        )
        export_command = (
            f'psql "${rollback_dsn_env}" -v ON_ERROR_STOP=1 -c '
            f'\\"\\copy ({export_query}) to \'{export_path}\' csv header\\"'
        )
        rebuild_checklist = [
            f"drop table if exists {target_relation} cascade;",
            f"recreate {target_relation} with the original root-table columns, but force repo_id text not null referencing {target_repositories_relation}(repo_id) wherever the table still carries repo scope.",
            f"preserve primary key ({spec['primary_key']}) and all non-repo identity columns exactly as they are today.",
            f"reload exported rows from {export_path} after the repo_id backfill projection succeeds.",
            "apply the separate root-constraints helper before moving to workflow-child tables.",
        ]
        validation_queries = [
            f"select count(*) as row_count from {target_relation};",
            f"select count(*) as null_repo_id_count from {target_relation} where repo_id is null or btrim(repo_id) = '';",
            "\n".join(
                [
                    f"select t.{spec['primary_key']}, t.repo_name, t.repo_id as table_repo_id, r.repo_id as repository_repo_id",
                    f"from {target_relation} as t",
                    f"join {target_repositories_relation} as r on r.repo_name = t.repo_name",
                    "where t.repo_id is distinct from r.repo_id",
                    f"order by t.{spec['primary_key']};",
                ]
            ),
        ]
        table_plans.append(
            {
                "table": spec["table"],
                "primary_key": spec["primary_key"],
                "columns": columns,
                "repo_name_unique": spec["repo_name_unique"],
                "source_relation": source_relation,
                "target_relation": target_relation,
                "export_path": export_path,
                "export_query": export_query,
                "export_command": export_command,
                "rebuild_checklist": rebuild_checklist,
                "validation_queries": validation_queries,
                "notes": spec["notes"],
            }
        )
    follow_up_notes = [
        "Run this helper after the content-plane repo-scoped slices land so workflow roots can inherit canonical repo_id from rebuilt repositories, lines, snapshots, and packs.",
        "This helper focuses on the root-table reload shape and repo_id backfill path; repo-aware uniqueness/index DDL is rendered separately by the root-constraints helper.",
        "Do not rebuild workflow-child tables until every root table in this helper validates with repo_id populated and repo_name/repo_id alignment intact.",
    ]
    return {
        "active_dsn_env": active_dsn_env,
        "rollback_dsn_env": rollback_dsn_env,
        "source_repositories_relation": source_repositories_relation,
        "target_repositories_relation": target_repositories_relation,
        "export_dir": export_dir,
        "tables": table_plans,
        "follow_up_notes": follow_up_notes,
    }


def format_plan(payload: dict[str, Any]) -> str:
    lines = [
        f"Rollback source DSN env: ${payload['rollback_dsn_env']}",
        f"Active target DSN env: ${payload['active_dsn_env']}",
        f"Export dir: {payload['export_dir']}",
    ]
    for table in payload["tables"]:
        lines.extend(
            [
                "",
                f"[{table['table']}]",
                f"Source: {table['source_relation']}",
                f"Target: {table['target_relation']}",
                f"Primary key: {table['primary_key']}",
                f"Export path: {table['export_path']}",
                "Rollback export command:",
                table["export_command"],
                "Rebuild checklist:",
            ]
        )
        lines.extend(f"- {step}" for step in table["rebuild_checklist"])
        lines.append("Validation queries:")
        lines.extend(table["validation_queries"])
        lines.append("Notes:")
        lines.extend(f"- {note}" for note in table["notes"])
    lines.extend(["", "Follow-up notes:"])
    lines.extend(f"- {note}" for note in payload["follow_up_notes"])
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rollback-dsn-env", default=DEFAULT_ROLLBACK_DSN_ENV, help=f"Environment variable name that holds the rollback database DSN. Default: {DEFAULT_ROLLBACK_DSN_ENV}.")
    parser.add_argument("--active-dsn-env", default=DEFAULT_ACTIVE_DSN_ENV, help=f"Environment variable name that holds the active database DSN. Default: {DEFAULT_ACTIVE_DSN_ENV}.")
    parser.add_argument("--source-schema", default="public", help="Rollback/source schema name. Default: public.")
    parser.add_argument("--target-schema", default="public", help="Active target schema name. Default: public.")
    parser.add_argument("--repositories-table", default="repositories", help="Repositories table name in both source and target schemas. Default: repositories.")
    parser.add_argument("--export-dir", default=DEFAULT_EXPORT_DIR, help=f"Directory used for rendered CSV export paths. Default: {DEFAULT_EXPORT_DIR}.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of text.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    payload = build_workflow_roots_rebuild_plan(
        rollback_dsn_env=args.rollback_dsn_env,
        active_dsn_env=args.active_dsn_env,
        source_schema=args.source_schema,
        target_schema=args.target_schema,
        repositories_table=args.repositories_table,
        export_dir=args.export_dir,
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
