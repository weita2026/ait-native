#!/usr/bin/env python3
"""Render the Wave 5 rollback cleanup plan."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any

DEFAULT_ROLLBACK_DATABASE = "ait_native_old"


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def build_repo_id_cleanup_plan(*, rollback_database: str = DEFAULT_ROLLBACK_DATABASE, active_database: str = "ait_native") -> dict[str, Any]:
    gates = [
        "row-count validation bundle is green for every rebuilt table",
        "repo-aware smoke checks are green for repository, content history, workflow queue, and session/job surfaces",
        "operator explicitly closes the frozen-write rollback window",
        "no consumer still requires the rollback database for forensic comparison",
    ]
    cleanup_commands = [
        f"dropdb --if-exists {rollback_database}",
        "# or, if the rollback copy is represented by *_old tables inside the active database, drop those tables only after the same gates pass",
        "# example: drop table if exists public.repositories_old cascade;",
    ]
    verification_queries = [
        f"select datname from pg_database where datname = '{rollback_database}';",
        "select schemaname, tablename from pg_tables where tablename like '%_old' order by schemaname, tablename;",
    ]
    notes = [
        f"Treat {rollback_database} as immutable until every validation and smoke-check gate is explicitly closed.",
        f"Cleanup is the last step; do not delete rollback artifacts while {active_database} is still in a tentative cutover state.",
        "Record the operator, timestamp, and approval evidence for rollback cleanup in the same change log or runbook packet that captured the freeze window.",
    ]
    return {
        "active_database": active_database,
        "rollback_database": rollback_database,
        "gates": gates,
        "cleanup_commands": cleanup_commands,
        "verification_queries": verification_queries,
        "notes": notes,
    }


def format_plan(payload: dict[str, Any]) -> str:
    lines = [f"Active database: {payload['active_database']}", f"Rollback database: {payload['rollback_database']}", "", "Gates:"]
    lines.extend(f"- {gate}" for gate in payload["gates"])
    lines.extend(["", "Cleanup commands:"])
    lines.extend(payload["cleanup_commands"])
    lines.extend(["", "Verification queries:"])
    lines.extend(payload["verification_queries"])
    lines.extend(["", "Notes:"])
    lines.extend(f"- {note}" for note in payload["notes"])
    return "\n".join(lines)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rollback-database", default=DEFAULT_ROLLBACK_DATABASE, help=f"Rollback database name. Default: {DEFAULT_ROLLBACK_DATABASE}.")
    parser.add_argument("--active-database", default="ait_native", help="Active database name. Default: ait_native.")
    parser.add_argument("--json", action="store_true", help="Emit JSON instead of text.")
    parser.add_argument("--output", help="Optional output path. Defaults to stdout.")
    return parser


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    payload = build_repo_id_cleanup_plan(
        rollback_database=args.rollback_database,
        active_database=args.active_database,
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
