#!/usr/bin/env python3
"""Render the Wave 4 payload-lineage audit plan."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def _json_dump(payload: Any) -> str:
    return json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False)


def build_payload_lineage_audit(*, schema: str = "public") -> dict[str, Any]:
    audits = [
        {
            "surface": "jobs.payload_json",
            "query": "\n".join(
                [
                    "select job_id, repo_id, payload_json",
                    f"from {schema}.jobs",
                    "where payload_json::jsonb ? 'repo_name'",
                    "  and coalesce(payload_json::jsonb ->> 'repo_id', '') = ''",
                    "order by job_id;",
                ]
            ),
            "purpose": "Find queued jobs whose payload still mentions repo_name but omits canonical repo_id.",
        },
        {
            "surface": "jobs.result_json",
            "query": "\n".join(
                [
                    "select job_id, repo_id, result_json",
                    f"from {schema}.jobs",
                    "where result_json::jsonb ? 'repo_name'",
                    "  and coalesce(result_json::jsonb ->> 'repo_id', '') = ''",
                    "order by job_id;",
                ]
            ),
            "purpose": "Catch worker results that still serialize only repo_name after the queue layer becomes repo_id-canonical.",
        },
        {
            "surface": "session_events.payload_json",
            "query": "\n".join(
                [
                    "select session_id, sequence, payload_json",
                    f"from {schema}.session_events",
                    "where payload_json::jsonb ? 'repo_name'",
                    "  and coalesce(payload_json::jsonb ->> 'repo_id', '') = ''",
                    "order by session_id, sequence;",
                ]
            ),
            "purpose": "Verify live session event payloads carry the same canonical repo scope as the rebuilt sessions roots.",
        },
        {
            "surface": "planning_session_events.payload_json",
            "query": "\n".join(
                [
                    "select planning_session_id, sequence, payload_json",
                    f"from {schema}.planning_session_events",
                    "where payload_json::jsonb ? 'repo_name'",
                    "  and coalesce(payload_json::jsonb ->> 'repo_id', '') = ''",
                    "order by planning_session_id, sequence;",
                ]
            ),
            "purpose": "Check planning-event payload lineage before downstream read models assume repo_id is always present.",
        },
        {
            "surface": "land_requests.result_json",
            "query": "\n".join(
                [
                    "select submission_id, repo_id, result_json",
                    f"from {schema}.land_requests",
                    "where result_json::jsonb ? 'repo_name'",
                    "  and coalesce(result_json::jsonb ->> 'repo_id', '') = ''",
                    "order by submission_id;",
                ]
            ),
            "purpose": "Confirm land-path result payloads keep repo_id visible for queue/read-model consumers.",
        },
    ]
    notes = [
        "Run these audits after the workflow-children reload helper so parent and child tables already expose canonical repo_id columns.",
        "If any row still emits repo_name without repo_id, patch the serializer or read-model surface before closing the cutover window.",
        "Queue summaries, job dashboards, and workflow detail read models should treat repo_id as canonical even if repo_name remains present for human readability.",
    ]
    return {"schema": schema, "audits": audits, "notes": notes}


def format_audit(payload: dict[str, Any]) -> str:
    lines = [f"Schema: {payload['schema']}"]
    for audit in payload["audits"]:
        lines.extend(["", f"[{audit['surface']}]", audit["purpose"], audit["query"]])
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
    payload = build_payload_lineage_audit(schema=args.schema)
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
