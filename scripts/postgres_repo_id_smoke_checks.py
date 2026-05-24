#!/usr/bin/env python3
"""Render the Wave 5 repo-aware smoke checks."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def build_repo_id_smoke_checks(*, schema: str = "public") -> dict[str, Any]:
    checks = [
        {
            "surface": "repository + line directory",
            "query": f"select r.repo_id, r.repo_name, l.line_name from {schema}.repositories r join {schema}.lines l on l.repo_id = r.repo_id order by r.repo_name, l.line_name limit 50;",
            "purpose": "Confirm repo/line surfaces resolve from repo_id and still render operator-readable repo_name values.",
        },
        {
            "surface": "snapshot history",
            "query": f"select s.repo_id, s.repo_name, s.line_name, s.snapshot_id, s.parent_snapshot_id from {schema}.snapshots s order by s.created_at desc limit 50;",
            "purpose": "Verify snapshot lineage and line_name reads survive the repo_id cutover.",
        },
        {
            "surface": "pack inventory",
            "query": f"select p.repo_id, p.repo_name, p.pack_id, p.status, p.pack_path from {schema}.packs p order by p.created_at desc limit 50;",
            "purpose": "Confirm pack metadata and operator-readable paths still resolve for repo-scoped history tables.",
        },
        {
            "surface": "workflow queue",
            "query": f"select c.change_id, c.repo_id, t.task_id, t.repo_id as task_repo_id, p.patchset_id from {schema}.changes c join {schema}.tasks t on t.task_id = c.task_id left join {schema}.patchsets p on p.change_id = c.change_id order by c.updated_at desc limit 50;",
            "purpose": "Check core workflow list surfaces still agree on canonical repo scope after root/child rebuilds.",
        },
        {
            "surface": "session + job surfaces",
            "query": f"select s.session_id, s.repo_id, j.job_id, j.repo_id as job_repo_id from {schema}.sessions s left join {schema}.jobs j on j.repo_id = s.repo_id order by s.updated_at desc limit 50;",
            "purpose": "Verify session and worker queue surfaces can still pivot by repo_id.",
        },
    ]
    notes = [
        "Run these smoke checks against the active database after row-count validation passes and before rollback cleanup begins.",
        "If a surface still relies on repo_name-only joins, stop and fix the reader before deleting the rollback source.",
        "These checks are meant to be operator-visible sanity checks, not a substitute for the per-table validation bundle.",
    ]
    return {"schema": schema, "checks": checks, "notes": notes}


def format_checks(payload: dict[str, Any]) -> str:
    lines = [f"Schema: {payload['schema']}"]
    for check in payload["checks"]:
        lines.extend(["", f"[{check['surface']}]", check["purpose"], check["query"]])
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
    payload = build_repo_id_smoke_checks(schema=args.schema)
    text = _json_dump(payload) if args.json else format_checks(payload)
    if args.output:
        output_path = Path(args.output).expanduser()
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(text + ("" if text.endswith("\n") else "\n"), encoding="utf-8")
    else:
        print(text)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
