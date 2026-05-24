from __future__ import annotations

import json
from pathlib import Path

from typer.testing import CliRunner

from ait.cli import app
from ait.local_first_final_land_benchmark import (
    evaluate_local_first_final_land_manifest,
    render_local_first_final_land_markdown,
)


runner = CliRunner()


def test_local_first_final_land_benchmark_supports_single_worker_recovery() -> None:
    manifest = {
        "benchmark_id": "unit-local-first-final-land",
        "candidate_mode": "ait_dag_local_first_final_land_e2e",
        "minimum_comparable_runs": 2,
        "workloads": [
            {
                "workload_id": "shared-session-dag",
                "category": "long",
                "runs": [
                    {
                        "run_id": "fresh-land",
                        "mode": "ait_dag_local_first_final_land_e2e",
                        "landed": True,
                        "operator_recovery_required": False,
                        "worker_session_count": 1,
                        "remote_change_count": 1,
                        "patchset_count": 1,
                        "elapsed_seconds": 410.0,
                        "stale_preflight": "fresh",
                        "stale_recovery_attempted": False,
                        "stale_recovery_succeeded": False,
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 1000, "completion_tokens": 500},
                    },
                    {
                        "run_id": "stale-recovered",
                        "mode": "ait_dag_local_first_final_land_e2e",
                        "landed": True,
                        "operator_recovery_required": False,
                        "worker_session_count": 1,
                        "remote_change_count": 1,
                        "patchset_count": 2,
                        "elapsed_seconds": 430.0,
                        "stale_preflight": "stale",
                        "stale_recovery_attempted": True,
                        "stale_recovery_succeeded": True,
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 1100, "completion_tokens": 500},
                    },
                ],
            }
        ],
    }

    payload = evaluate_local_first_final_land_manifest(manifest)

    assert payload["evidence_type"] == "measured"
    assert payload["aggregate"]["verdict"] == "supported"
    assert payload["aggregate"]["landed_rate"] == 100.0
    assert payload["aggregate"]["single_worker_success_rate"] == 100.0
    assert payload["aggregate"]["stale_recovery_success_rate"] == 100.0
    report = render_local_first_final_land_markdown(payload)
    assert "ait_dag_local_first_final_land_e2e" in report
    assert "stale-recovered" in report


def test_local_first_final_land_benchmark_marks_operator_recovery_as_mixed() -> None:
    manifest = {
        "benchmark_id": "unit-local-first-final-land-mixed",
        "candidate_mode": "ait_dag_local_first_final_land_e2e",
        "minimum_comparable_runs": 1,
        "workloads": [
            {
                "workload_id": "shared-session-dag",
                "category": "long",
                "runs": [
                    {
                        "run_id": "operator-finish",
                        "landed": True,
                        "operator_recovery_required": True,
                        "worker_session_count": 2,
                        "remote_change_count": 2,
                        "patchset_count": 2,
                        "stale_preflight": "stale",
                        "stale_recovery_attempted": True,
                        "stale_recovery_succeeded": False,
                    }
                ],
            }
        ],
    }

    payload = evaluate_local_first_final_land_manifest(manifest)

    assert payload["evidence_type"] == "operational"
    assert payload["aggregate"]["verdict"] == "mixed"
    assert payload["aggregate"]["operator_recovery_required_count"] == 1
    assert payload["aggregate"]["single_worker_success_count"] == 0


def test_benchmark_local_first_final_land_cli_writes_reports(tmp_path: Path) -> None:
    with runner.isolated_filesystem(temp_dir=tmp_path):
        manifest = {
            "benchmark_id": "unit-local-first-cli",
            "candidate_mode": "ait_dag_local_first_final_land_e2e",
            "minimum_comparable_runs": 1,
            "workloads": [
                {
                    "workload_id": "multi-transport",
                    "category": "long",
                    "runs": [
                        {
                            "run_id": "run-1",
                            "landed": True,
                            "operator_recovery_required": False,
                            "worker_session_count": 1,
                            "remote_change_count": 1,
                            "patchset_count": 1,
                            "stale_preflight": "fresh",
                            "stale_recovery_attempted": False,
                            "stale_recovery_succeeded": False,
                            "usage_kind": "measured",
                            "usage": {"prompt_tokens": 900, "completion_tokens": 300},
                        }
                    ],
                }
            ],
        }
        Path("manifest.json").write_text(json.dumps(manifest), encoding="utf-8")

        result = runner.invoke(
            app,
            [
                "benchmark",
                "local-first-final-land",
                "--manifest",
                "manifest.json",
                "--output-json",
                "out/report.json",
                "--output-markdown",
                "out/report.md",
                "--json",
            ],
            catch_exceptions=False,
        )

        assert result.exit_code == 0, result.stdout
        payload = json.loads(result.stdout)
        assert payload["aggregate"]["verdict"] == "supported"
        assert Path("out/report.json").exists()
        assert "unit-local-first-cli" in Path("out/report.md").read_text(encoding="utf-8")
