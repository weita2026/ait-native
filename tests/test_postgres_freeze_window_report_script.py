from __future__ import annotations

import importlib.util
import json
import sys
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = ROOT / "scripts" / "postgres_freeze_window_report.py"
SPEC = importlib.util.spec_from_file_location("postgres_freeze_window_report", SCRIPT_PATH)
assert SPEC is not None and SPEC.loader is not None
freeze_script = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = freeze_script
SPEC.loader.exec_module(freeze_script)


def test_build_freeze_window_report_flags_missing_gates() -> None:
    payload = freeze_script.build_freeze_window_report(
        owner="weita",
        oncall="ops",
        window_id="MW-42",
        notes="waiting for web stop",
        freeze_started_at=datetime(2026, 5, 11, 4, 30, tzinfo=timezone.utc),
        writes_blocked=True,
        manual_writes_blocked=True,
        workers_stopped=True,
        server_stopped=True,
        web_stopped=False,
        command_results=[
            {"name": "queue_summary", "ok": True, "error": None, "payload": {"ready_to_land": 0}},
            {"name": "postgres_doctor", "ok": False, "error": "connect failed", "payload": None},
        ],
    )

    assert payload["go_no_go_ready"] is False
    assert payload["command_failures"] == ["postgres_doctor"]
    assert any("ait-web" in row for row in payload["recommendations"])
    assert any("postgres_doctor" in row for row in payload["recommendations"])


def test_main_writes_json_report(monkeypatch, tmp_path: Path) -> None:
    monkeypatch.setattr(
        freeze_script,
        "capture_freeze_window_report",
        lambda **kwargs: {
            "generated_at": "2026-05-11T04:45:00+00:00",
            "window": {
                "owner": kwargs["owner"],
                "oncall": kwargs["oncall"],
                "window_id": kwargs["window_id"],
                "freeze_started_at": "2026-05-11T04:44:00+00:00",
                "notes": kwargs["notes"],
            },
            "gate_checks": {
                "writes_blocked": True,
                "manual_writes_blocked": True,
                "workers_stopped": True,
                "server_stopped": True,
                "web_stopped": True,
            },
            "commands": [{"name": "queue_summary", "ok": True, "error": None, "payload": {}}],
            "command_failures": [],
            "go_no_go_ready": True,
            "recommendations": ["ready"],
        },
    )

    output_path = tmp_path / "freeze-window.json"
    exit_code = freeze_script.main(
        [
            "--owner",
            "weita",
            "--oncall",
            "ops",
            "--window-id",
            "MW-42",
            "--notes",
            "freeze confirmed",
            "--writes-blocked",
            "--manual-writes-blocked",
            "--workers-stopped",
            "--server-stopped",
            "--web-stopped",
            "--json",
            "--output",
            str(output_path),
        ]
    )

    assert exit_code == 0
    written = json.loads(output_path.read_text(encoding="utf-8"))
    assert written["go_no_go_ready"] is True
    assert written["window"]["owner"] == "weita"
