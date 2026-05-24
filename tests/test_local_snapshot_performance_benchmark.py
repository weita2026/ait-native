from __future__ import annotations

import json
from pathlib import Path

from typer.testing import CliRunner

from ait.cli import app
from ait.local_snapshot_performance_benchmark import (
    evaluate_local_snapshot_performance_manifest,
    render_local_snapshot_performance_markdown,
)


runner = CliRunner()


def _complete_manifest() -> dict:
    return {
        "benchmark_id": "unit-local-snapshot-performance",
        "candidate_mode": "ait_local",
        "baseline_mode": "git_baseline",
        "minimum_comparable_runs": 1,
        "workloads": [
            {
                "workload_id": "ai-radio-engine",
                "title": "AI_Radio_Engine",
                "category": "large_workspace",
                "workspace_profile": {
                    "workspace_name": "AI_Radio_Engine",
                    "file_count": 1349,
                    "total_bytes": 378549070,
                },
                "runs": [
                    {"run_id": "ait-status-first", "mode": "ait_local", "operation": "workspace_status", "phase": "first_run", "elapsed_seconds": 6.2},
                    {"run_id": "git-status-first", "mode": "git_baseline", "operation": "workspace_status", "phase": "first_run", "elapsed_seconds": 1.8},
                    {"run_id": "ait-status-warm", "mode": "ait_local", "operation": "workspace_status", "phase": "warm_noop", "elapsed_seconds": 2.9},
                    {"run_id": "git-status-warm", "mode": "git_baseline", "operation": "workspace_status", "phase": "warm_noop", "elapsed_seconds": 0.7},
                    {"run_id": "ait-snapshot-first", "mode": "ait_local", "operation": "snapshot_create", "phase": "first_run", "elapsed_seconds": 67.15},
                    {"run_id": "git-snapshot-first", "mode": "git_baseline", "operation": "snapshot_create", "phase": "first_run", "elapsed_seconds": 12.19},
                    {"run_id": "ait-snapshot-warm", "mode": "ait_local", "operation": "snapshot_create", "phase": "warm_noop", "elapsed_seconds": 18.4},
                    {"run_id": "git-snapshot-warm", "mode": "git_baseline", "operation": "snapshot_create", "phase": "warm_noop", "elapsed_seconds": 4.6},
                    {"run_id": "ait-push-first", "mode": "ait_local", "operation": "push", "phase": "first_run", "elapsed_seconds": 24.0, "healthz_ok": True},
                    {"run_id": "git-push-first", "mode": "git_baseline", "operation": "push", "phase": "first_run", "elapsed_seconds": 10.4, "healthz_ok": True},
                    {"run_id": "ait-push-warm", "mode": "ait_local", "operation": "push", "phase": "warm_noop", "elapsed_seconds": 5.9, "healthz_ok": True},
                    {"run_id": "git-push-warm", "mode": "git_baseline", "operation": "push", "phase": "warm_noop", "elapsed_seconds": 2.1, "healthz_ok": True},
                ],
            }
        ],
    }


def _budgeted_manifest() -> dict:
    manifest = _complete_manifest()
    manifest["budget_targets"] = [
        {
            "operation": "workspace_status",
            "phase": "warm_noop",
            "max_candidate_elapsed_seconds": 3.0,
            "notes": "Warm status should stay in the low seconds range.",
        },
        {
            "operation": "snapshot_create",
            "phase": "first_run",
            "max_candidate_vs_baseline_ratio": 4.0,
            "notes": "First snapshot should trend toward <=4x the Git baseline.",
        },
        {
            "operation": "push",
            "phase": "first_run",
            "require_push_health_green": True,
            "notes": "Representative pushes must keep /healthz green.",
        },
    ]
    return manifest


def test_local_snapshot_performance_benchmark_ready_for_budgeting() -> None:
    payload = evaluate_local_snapshot_performance_manifest(_complete_manifest())

    assert payload["aggregate"]["verdict"] == "ready_for_budgeting"
    assert payload["aggregate"]["comparable_case_count"] == 6
    assert payload["aggregate"]["push_health_green_case_count"] == 2
    case_map = {(row["operation"], row["phase"]): row for row in payload["case_summaries"]}
    assert case_map[("snapshot_create", "first_run")]["candidate_vs_baseline_ratio"] == round(67.15 / 12.19, 6)
    report = render_local_snapshot_performance_markdown(payload)
    assert "Case Summary" in report
    assert "AI_Radio_Engine" in report


def test_local_snapshot_performance_benchmark_marks_missing_cases_incomplete() -> None:
    manifest = _complete_manifest()
    manifest["workloads"][0]["runs"] = [
        run for run in manifest["workloads"][0]["runs"] if not (run["operation"] == "push" and run["phase"] == "warm_noop")
    ]

    payload = evaluate_local_snapshot_performance_manifest(manifest)

    assert payload["aggregate"]["verdict"] == "incomplete"
    assert payload["aggregate"]["comparable_case_count"] == 5


def test_local_snapshot_performance_benchmark_reports_budget_failures() -> None:
    payload = evaluate_local_snapshot_performance_manifest(_budgeted_manifest())

    assert payload["aggregate"]["verdict"] == "ready_for_budgeting"
    assert payload["aggregate"]["budget_verdict"] == "fail"
    assert payload["budget_summary"]["tracked_case_count"] == 3
    assert payload["budget_summary"]["passed_case_count"] == 2
    assert payload["budget_summary"]["failed_case_count"] == 1
    case_map = {(row["operation"], row["phase"]): row for row in payload["case_summaries"]}
    assert case_map[("workspace_status", "warm_noop")]["budget_result"]["verdict"] == "pass"
    assert case_map[("push", "first_run")]["budget_result"]["verdict"] == "pass"
    assert case_map[("snapshot_create", "first_run")]["budget_result"]["verdict"] == "fail"
    report = render_local_snapshot_performance_markdown(payload)
    assert "## Performance Budget" in report
    assert "Budget verdict: `fail`" in report


def test_local_snapshot_performance_benchmark_rejects_budget_targets_outside_required_cases() -> None:
    manifest = _budgeted_manifest()
    manifest["required_cases"] = [
        {"operation": "workspace_status", "phase": "warm_noop"},
        {"operation": "push", "phase": "first_run"},
    ]

    try:
        evaluate_local_snapshot_performance_manifest(manifest)
    except ValueError as exc:
        assert "budget_targets must only reference required_cases" in str(exc)
    else:
        raise AssertionError("expected required-case validation to fail")


def test_benchmark_local_snapshot_performance_cli_writes_reports(tmp_path: Path) -> None:
    with runner.isolated_filesystem(temp_dir=tmp_path):
        Path("manifest.json").write_text(json.dumps(_complete_manifest()), encoding="utf-8")

        result = runner.invoke(
            app,
            [
                "benchmark",
                "local-snapshot-performance",
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
        assert payload["aggregate"]["verdict"] == "ready_for_budgeting"
        assert Path("out/report.json").exists()
        assert "unit-local-snapshot-performance" in Path("out/report.md").read_text(encoding="utf-8")


def test_benchmark_local_snapshot_performance_cli_writes_budget_summary(tmp_path: Path) -> None:
    with runner.isolated_filesystem(temp_dir=tmp_path):
        Path("manifest.json").write_text(json.dumps(_budgeted_manifest()), encoding="utf-8")

        result = runner.invoke(
            app,
            [
                "benchmark",
                "local-snapshot-performance",
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
        assert payload["budget_summary"]["verdict"] == "fail"
        assert "## Performance Budget" in Path("out/report.md").read_text(encoding="utf-8")
