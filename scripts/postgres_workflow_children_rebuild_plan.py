#!/usr/bin/env python3
"""Render the Wave 4 workflow-children rebuild rehearsal plan."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

DEFAULT_ACTIVE_DSN_ENV = "AIT_NATIVE_SERVER_POSTGRES_DSN"
DEFAULT_ROLLBACK_DSN_ENV = "AIT_ROLLBACK_POSTGRES_DSN"
DEFAULT_EXPORT_DIR = "workflow_children_wave4"
CHILD_TABLE_SPECS = [
    {"table": "plan_revisions", "order_by": "plan_id, revision_number", "parent_contract": "plan_id -> plans.plan_id", "scope_mode": "parent-derived", "repo_id_resolution": None, "notes": ["Parent plan row carries canonical repo scope after Wave 3."]},
    {"table": "plan_revision_blobs", "order_by": "created_at, plan_revision_id", "parent_contract": "plan_revision_id -> plan_revisions.plan_revision_id", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.plan_revisions pr on pr.plan_revision_id = t.plan_revision_id join public.plans p on p.plan_id = pr.plan_id to fill repo_id from p.repo_id when needed", "notes": ["Keep repo_name only as temporary compatibility metadata if still required during the cutover window."]},
    {"table": "plan_revision_artifacts", "order_by": "updated_at, plan_revision_id, artifact_path", "parent_contract": "plan_revision_id -> plan_revisions.plan_revision_id", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.plan_revisions pr on pr.plan_revision_id = t.plan_revision_id join public.plans p on p.plan_id = pr.plan_id to fill repo_id from p.repo_id when needed", "notes": ["Artifact paths stay stable; only canonical repo scope changes."]},
    {"table": "planning_session_events", "order_by": "planning_session_id, sequence", "parent_contract": "planning_session_id -> planning_sessions.planning_session_id", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.planning_sessions ps on ps.planning_session_id = t.planning_session_id to fill repo_id from ps.repo_id", "notes": ["Payload JSON stays untouched in this slice; payload lineage audits follow separately."]},
    {"table": "session_events", "order_by": "session_id, sequence", "parent_contract": "session_id -> sessions.session_id", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.sessions s on s.session_id = t.session_id to fill repo_id from s.repo_id", "notes": ["Keep sequence ordering and actor fields unchanged." ]},
    {"table": "session_checkpoints", "order_by": "session_id, created_at", "parent_contract": "session_id -> sessions.session_id", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.sessions s on s.session_id = t.session_id to fill repo_id from s.repo_id", "notes": ["Resume payload JSON stays opaque; payload lineage is audited separately."]},
    {"table": "patchsets", "order_by": "change_id, patchset_number", "parent_contract": "change_id -> changes.change_id", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.changes c on c.change_id = t.change_id to fill repo_id from c.repo_id", "notes": ["Patchset identity remains change_id + patchset_number."]},
    {"table": "review_requests", "order_by": "change_id, patchset_id, review_request_id", "parent_contract": "change_id/patchset_id -> changes/patchsets", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.changes c on c.change_id = t.change_id or join public.patchsets p on p.patchset_id = t.patchset_id to fill repo_id", "notes": ["Reviewer routing remains intact while repo scope becomes canonical."]},
    {"table": "reviews", "order_by": "change_id, patchset_id, review_id", "parent_contract": "change_id/patchset_id -> changes/patchsets", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.changes c on c.change_id = t.change_id or join public.patchsets p on p.patchset_id = t.patchset_id to fill repo_id", "notes": ["Historical review evidence must keep ordering and reviewer identity untouched."]},
    {"table": "attestations", "order_by": "updated_at, patchset_id", "parent_contract": "patchset_id -> patchsets.patchset_id", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.patchsets p on p.patchset_id = t.patchset_id to fill repo_id from p.repo_id", "notes": ["Do not rewrite attestation detail/provenance JSON in this slice."]},
    {"table": "policy_decisions", "order_by": "patchset_id, policy_decision_id", "parent_contract": "patchset_id -> patchsets.patchset_id", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.patchsets p on p.patchset_id = t.patchset_id to fill repo_id from p.repo_id", "notes": ["Lane/decision history stays append-only."]},
    {"table": "waivers", "order_by": "patchset_id, created_at", "parent_contract": "patchset_id -> patchsets.patchset_id", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.patchsets p on p.patchset_id = t.patchset_id to fill repo_id from p.repo_id", "notes": ["Waiver identity and expiry timestamps stay unchanged."]},
    {"table": "land_requests", "order_by": "change_id, created_at", "parent_contract": "change_id/patchset_id -> changes/patchsets", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.changes c on c.change_id = t.change_id or join public.patchsets p on p.patchset_id = t.patchset_id to fill repo_id", "notes": ["Leave result_json untouched; audit payload lineage separately." ]},
    {"table": "stack_changes", "order_by": "stack_id, position", "parent_contract": "stack_id/change_id -> stacks/changes", "scope_mode": "explicit-repo-id", "repo_id_resolution": "join public.stacks s on s.stack_id = t.stack_id to fill repo_id from s.repo_id", "notes": ["Preserve stack order and change membership semantics." ]},
    {"table": "authority_nodes", "order_by": "authority_map_id, parent_node_id, sort_index", "parent_contract": "authority_map_id -> authority_maps.authority_map_id", "scope_mode": "parent-derived", "repo_id_resolution": None, "notes": ["Authority scope derives from authority_maps; node rows do not need their own repo_id column." ]},
    {"table": "authority_mutations", "order_by": "authority_map_id, created_at", "parent_contract": "authority_map_id/authority_node_id -> authority_maps/authority_nodes", "scope_mode": "parent-derived", "repo_id_resolution": None, "notes": ["Mutation lineage stays attached to canonical authority_map scope." ]},
]


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def _qualified_table(schema: str, table: str) -> str:
    return f'"{schema}"."{table}"'


def build_workflow_children_rebuild_plan(
    *,
    rollback_dsn_env: str = DEFAULT_ROLLBACK_DSN_ENV,
    active_dsn_env: str = DEFAULT_ACTIVE_DSN_ENV,
    source_schema: str = "public",
    target_schema: str = "public",
    export_dir: str = DEFAULT_EXPORT_DIR,
) -> dict[str, Any]:
    tables: list[dict[str, Any]] = []
    for spec in CHILD_TABLE_SPECS:
        source_relation = _qualified_table(source_schema, spec["table"])
        target_relation = _qualified_table(target_schema, spec["table"])
        export_path = f"{export_dir}/{spec['table']}.csv"
        export_query = f"select * from {source_relation} order by {spec['order_by']}"
        export_command = (
            f'psql "${rollback_dsn_env}" -v ON_ERROR_STOP=1 -c '
            f'\\"\\copy ({export_query}) to \'{export_path}\' csv header\\"'
        )
        rebuild_checklist = [
            f"drop/recreate {target_relation} only after its parent contract ({spec['parent_contract']}) is already canonical in the active database.",
            f"reload rows from {export_path} while preserving the existing parent/child ordering on {spec['order_by']}.",
        ]
        if spec["repo_id_resolution"]:
            rebuild_checklist.append(f"resolve missing repo_id values by: {spec['repo_id_resolution']}.")
            rebuild_checklist.append("alter the explicit repo_id column to not null only after the resolution query comes back clean.")
        else:
            rebuild_checklist.append("keep repo scope parent-derived; do not invent a new child repo_id column when the parent already carries canonical scope.")
        validation_queries = [
            f"select count(*) as row_count from {target_relation};",
            f"select * from {target_relation} limit 1;",
        ]
        if spec["repo_id_resolution"]:
            validation_queries.append(f"select count(*) as null_repo_id_count from {target_relation} where repo_id is null or btrim(repo_id) = '';")
        tables.append(
            {
                "table": spec["table"],
                "parent_contract": spec["parent_contract"],
                "scope_mode": spec["scope_mode"],
                "source_relation": source_relation,
                "target_relation": target_relation,
                "export_path": export_path,
                "export_query": export_query,
                "export_command": export_command,
                "repo_id_resolution": spec["repo_id_resolution"],
                "rebuild_checklist": rebuild_checklist,
                "validation_queries": validation_queries,
                "notes": spec["notes"],
            }
        )
    notes = [
        "Run this helper only after the workflow-root rebuild and root-constraints helpers land so every parent table already has canonical repo_id authority.",
        "Use parent-derived scope for authority_nodes/authority_mutations and other lineage tables that do not need their own repo_id column.",
        "Follow with the payload-lineage audit so queue/job/session JSON surfaces also expose the same canonical repo scope.",
    ]
    return {
        "active_dsn_env": active_dsn_env,
        "rollback_dsn_env": rollback_dsn_env,
        "export_dir": export_dir,
        "tables": tables,
        "notes": notes,
    }


def format_plan(payload: dict[str, Any]) -> str:
    lines = [
        f"Rollback source DSN env: ${payload['rollback_dsn_env']}",
        f"Active target DSN env: ${payload['active_dsn_env']}",
        f"Export dir: {payload['export_dir']}",
    ]
    for table in payload["tables"]:
        lines.extend(["", f"[{table['table']}]", f"Parent contract: {table['parent_contract']}", f"Scope mode: {table['scope_mode']}", f"Source: {table['source_relation']}", f"Target: {table['target_relation']}", f"Export path: {table['export_path']}", "Rollback export command:", table["export_command"], "Rebuild checklist:"])
        lines.extend(f"- {step}" for step in table["rebuild_checklist"])
        lines.append("Validation queries:")
        lines.extend(table["validation_queries"])
        lines.append("Notes:")
        lines.extend(f"- {note}" for note in table["notes"])
    lines.extend(["", "Notes:"])
    lines.extend(f"- {note}" for note in payload["notes"])
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rollback-dsn-env", default=DEFAULT_ROLLBACK_DSN_ENV, help=f"Environment variable name that holds the rollback database DSN. Default: {DEFAULT_ROLLBACK_DSN_ENV}.")
    parser.add_argument("--active-dsn-env", default=DEFAULT_ACTIVE_DSN_ENV, help=f"Environment variable name that holds the active database DSN. Default: {DEFAULT_ACTIVE_DSN_ENV}.")
    parser.add_argument("--source-schema", default="public", help="Rollback/source schema name. Default: public.")
    parser.add_argument("--target-schema", default="public", help="Active target schema name. Default: public.")
    parser.add_argument("--export-dir", default=DEFAULT_EXPORT_DIR, help=f"Directory used for rendered CSV export paths. Default: {DEFAULT_EXPORT_DIR}.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of text.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    payload = build_workflow_children_rebuild_plan(
        rollback_dsn_env=args.rollback_dsn_env,
        active_dsn_env=args.active_dsn_env,
        source_schema=args.source_schema,
        target_schema=args.target_schema,
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
