from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_payload_lineage_audit.py"
SPEC = importlib.util.spec_from_file_location("postgres_payload_lineage_audit", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
plan_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = plan_script
SPEC.loader.exec_module(plan_script)


def test_build_audit_renders_queue_and_read_model_payload_checks() -> None:
    payload = plan_script.build_payload_lineage_audit(schema="control")

    audits = {entry["surface"]: entry for entry in payload["audits"]}
    assert "jobs.payload_json" in audits and "session_events.payload_json" in audits
    assert "repo_name" in audits["jobs.payload_json"]["query"]
    assert "repo_id" in audits["planning_session_events.payload_json"]["query"]
    assert "control.land_requests" in audits["land_requests.result_json"]["query"]
    assert any("Queue summaries" in note for note in payload["notes"])


def test_main_writes_json_report(tmp_path: Path) -> None:
    output_path = tmp_path / "payload-lineage-audit.json"
    exit_code = plan_script.main(["--json", "--output", str(output_path)])

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["schema"] == "public"
    assert len(written["audits"]) == 5
