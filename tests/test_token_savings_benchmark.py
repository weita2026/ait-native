from __future__ import annotations

import json
from pathlib import Path

import pytest
from typer.testing import CliRunner

from ait.cli import app
from ait.strict_rerun_builder import build_strict_rerun_fixture_bundle
from ait.token_benchmark import (
    evaluate_token_savings_manifest,
    extract_codex_token_usage,
    extract_codex_token_usage_bundle,
    inspect_token_savings_collection,
    render_token_savings_markdown,
)


runner = CliRunner()


def test_token_savings_benchmark_supports_measured_usage() -> None:
    manifest = {
        "benchmark_id": "unit-measured",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag"],
        "minimum_comparable_long_workloads": 1,
        "workloads": [
            {
                "workload_id": "long-demo",
                "category": "long",
                "runs": [
                    {
                        "run_id": "baseline",
                        "mode": "git_linear",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 1000, "completion_tokens": 500},
                    },
                    {
                        "run_id": "dag",
                        "mode": "ait_dag",
                        "usage_kind": "measured",
                        "usage": {"input_tokens": 600, "output_tokens": 300},
                    },
                ],
            }
        ],
    }

    payload = evaluate_token_savings_manifest(manifest)

    assert payload["evidence_type"] == "measured"
    assert payload["aggregate"]["verdict"] == "supported"
    assert payload["aggregate"]["claim_ready"] is True
    comparison = payload["workloads"][0]["comparisons"][0]
    assert comparison["baseline_median_total_tokens"] == 1500
    assert comparison["candidate_median_total_tokens"] == 900
    assert comparison["saving_percent"] == 40.0


def test_benchmark_token_savings_cli_writes_reports(tmp_path: Path) -> None:
    with runner.isolated_filesystem(temp_dir=tmp_path):
        manifest = {
            "benchmark_id": "unit-measured-cli-report",
            "baseline_mode": "git_linear",
            "candidate_modes": ["ait_dag"],
            "minimum_comparable_long_workloads": 1,
            "workloads": [
                {
                    "workload_id": "measured-report",
                    "category": "long",
                    "runs": [
                        {
                            "run_id": "baseline",
                            "mode": "git_linear",
                            "usage_kind": "measured",
                            "usage": {"prompt_tokens": 1000, "completion_tokens": 500},
                        },
                        {
                            "run_id": "dag",
                            "mode": "ait_dag",
                            "usage_kind": "measured",
                            "usage": {"prompt_tokens": 600, "completion_tokens": 300},
                        },
                    ],
                }
            ],
        }
        Path("manifest.json").write_text(json.dumps(manifest), encoding="utf-8")

        result = runner.invoke(
            app,
            [
                "benchmark",
                "token-savings",
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
        assert payload["evidence_type"] == "measured"
        assert payload["aggregate"]["verdict"] == "supported"
        assert payload["workloads"][0]["comparisons"][0]["saving_percent"] == 40.0
        assert Path("out/report.json").exists()
        assert "unit-measured-cli-report" in Path("out/report.md").read_text(encoding="utf-8")


def test_token_savings_benchmark_supports_custom_aggregate_candidate_mode() -> None:
    manifest = {
        "benchmark_id": "unit-remote-batch-measured",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag", "remote_batch"],
        "aggregate_candidate_mode": "remote_batch",
        "minimum_comparable_long_workloads": 1,
        "workloads": [
            {
                "workload_id": "solo-remote-batch",
                "category": "long",
                "runs": [
                    {
                        "run_id": "baseline",
                        "mode": "git_linear",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 1000, "completion_tokens": 500},
                    },
                    {
                        "run_id": "ait-dag",
                        "mode": "ait_dag",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 900, "completion_tokens": 500},
                    },
                    {
                        "run_id": "batch",
                        "mode": "remote_batch",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 500, "completion_tokens": 250},
                    },
                ],
            }
        ],
    }

    payload = evaluate_token_savings_manifest(manifest)

    assert payload["aggregate_candidate_mode"] == "remote_batch"
    assert payload["aggregate"]["aggregate_candidate_mode"] == "remote_batch"
    assert payload["aggregate"]["long_candidate_median_saving_percent"] == 50.0
    assert payload["aggregate"]["per_mode"]["ait_dag"]["median_saving_percent"] == 6.67
    assert payload["aggregate"]["per_mode"]["remote_batch"]["median_saving_percent"] == 50.0


def test_token_savings_benchmark_uses_directional_caveat_for_packet_recovery_profiles() -> None:
    manifest = {
        "benchmark_id": "unit-packet-recovery-directional",
        "baseline_mode": "git_linear",
        "candidate_modes": ["packet_candidate"],
        "aggregate_candidate_mode": "packet_candidate",
        "minimum_comparable_long_workloads": 1,
        "comparison_profiles": [
            {
                "profile_id": "packet-recovery",
                "profile_kind": "packet_recovery",
                "baseline_mode": "git_linear",
                "candidate_modes": ["packet_candidate"],
                "aggregate_candidate_mode": "packet_candidate",
                "minimum_comparable_long_workloads": 1,
                "claim_target": False,
            }
        ],
        "workloads": [
            {
                "workload_id": "beta-release",
                "category": "long",
                "runs": [
                    {
                        "run_id": "baseline",
                        "mode": "git_linear",
                        "quality": "passed",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 1000, "completion_tokens": 500},
                    },
                    {
                        "run_id": "planning",
                        "mode": "packet_candidate",
                        "quality": "passed",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 100, "completion_tokens": 50},
                    },
                ],
            }
        ],
    }

    payload = evaluate_token_savings_manifest(manifest)

    assert payload["aggregate"]["claim_target"] is False
    assert "Directional packet-only evidence only" in payload["aggregate"]["claim_caveat"]


def test_token_savings_benchmark_reports_known_confounders() -> None:
    manifest = {
        "benchmark_id": "unit-known-confounders",
        "description": "Provider-measured mixed-date comparison.",
        "known_confounders": [
            "reference rows were collected on an earlier Codex CLI revision",
            "",
            "candidate rows were collected with a restored packet compiler",
        ],
        "baseline_mode": "git_linear",
        "candidate_modes": ["planning_compiler_packet"],
        "aggregate_candidate_mode": "planning_compiler_packet",
        "minimum_comparable_long_workloads": 1,
        "comparison_profiles": [
            {
                "profile_id": "packet",
                "profile_kind": "packet_recovery",
                "baseline_mode": "git_linear",
                "candidate_modes": ["planning_compiler_packet"],
                "aggregate_candidate_mode": "planning_compiler_packet",
                "claim_target": False,
            }
        ],
        "workloads": [
            {
                "workload_id": "long-demo",
                "category": "long",
                "runs": [
                    {
                        "run_id": "baseline",
                        "mode": "git_linear",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 1000, "completion_tokens": 500},
                    },
                    {
                        "run_id": "candidate",
                        "mode": "planning_compiler_packet",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 200, "completion_tokens": 100},
                    },
                ],
            }
        ],
    }

    payload = evaluate_token_savings_manifest(manifest)
    report = render_token_savings_markdown(payload)

    assert payload["known_confounders"] == [
        "reference rows were collected on an earlier Codex CLI revision",
        "candidate rows were collected with a restored packet compiler",
    ]
    assert "## Known Confounders" in report
    assert "Provider-measured mixed-date comparison." in report
    assert "restored packet compiler" in report


def test_token_savings_benchmark_rejects_context_only_usage() -> None:
    manifest = {
        "benchmark_id": "unit-provider-measured-required",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag"],
        "minimum_comparable_long_workloads": 1,
        "workloads": [
            {
                "workload_id": "context-only",
                "category": "long",
                "runs": [
                    {"run_id": "baseline", "mode": "git_linear", "prompt_text": "a" * 400},
                    {"run_id": "dag", "mode": "ait_dag", "prompt_text": "b" * 200},
                ],
            }
        ],
    }

    with pytest.raises(ValueError, match="provider-measured token usage is required"):
        evaluate_token_savings_manifest(manifest)


def test_token_savings_status_reports_pending_measured_runs(tmp_path: Path) -> None:
    manifest = {
        "benchmark_id": "pending-measured",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag"],
        "workloads": [
            {
                "workload_id": "pending-workload",
                "category": "long",
                "runs": [
                    {
                        "run_id": "pending-run",
                        "mode": "git_linear",
                        "usage_kind": "measured",
                        "quality": "pending",
                        "usage": {"prompt_tokens": None, "completion_tokens": None},
                    },
                    {
                        "run_id": "ready-run",
                        "mode": "ait_dag",
                        "usage_kind": "measured",
                        "quality": "passed",
                        "usage": {"prompt_tokens": 10, "completion_tokens": 5},
                    },
                ],
            }
        ],
    }
    with runner.isolated_filesystem(temp_dir=tmp_path):
        Path("manifest.json").write_text(json.dumps(manifest), encoding="utf-8")
        result = runner.invoke(
            app,
            [
                "benchmark",
                "token-savings-status",
                "--manifest",
                "manifest.json",
                "--output-json",
                "status.json",
                "--json",
            ],
            catch_exceptions=False,
        )
        status_file_payload = json.loads(Path("status.json").read_text(encoding="utf-8"))

    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["summary"]["total_run_count"] == 2
    assert payload["summary"]["measured_ready_count"] == 1
    assert payload["summary"]["missing_usage_count"] == 1
    assert payload["summary"]["pending_quality_count"] == 1
    assert payload["summary"]["ready_to_report"] is False
    assert payload["runs"][0]["missing_reason"] == "missing_usage, quality_pending"
    assert status_file_payload["summary"] == payload["summary"]


def test_token_savings_status_requires_equivalent_completion(tmp_path: Path) -> None:
    manifest = {
        "benchmark_id": "pending-equivalent-completion",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag_local_first_final_land_packet"],
        "require_equivalent_completion": True,
        "workloads": [
            {
                "workload_id": "equivalent-gate",
                "category": "long",
                "runs": [
                    {
                        "run_id": "baseline-ready",
                        "mode": "git_linear",
                        "usage_kind": "measured",
                        "quality": "passed",
                        "completion_status": "equivalent",
                        "usage": {"prompt_tokens": 100, "completion_tokens": 40},
                    },
                    {
                        "run_id": "candidate-partial",
                        "mode": "ait_dag_local_first_final_land_packet",
                        "usage_kind": "measured",
                        "quality": "passed",
                        "completion_status": "partial",
                        "usage": {"prompt_tokens": 20, "completion_tokens": 5},
                    },
                ],
            }
        ],
    }
    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

    payload = inspect_token_savings_collection(manifest_path)

    assert payload["require_equivalent_completion"] is True
    assert payload["summary"]["measured_ready_count"] == 1
    assert payload["summary"]["ready_to_report"] is False
    candidate = next(run for run in payload["runs"] if run["run_id"] == "candidate-partial")
    assert candidate["completion_status"] == "partial"
    assert candidate["completion_equivalent"] is False
    assert candidate["missing_reason"] == "completion_not_equivalent"


def test_token_savings_benchmark_requires_equivalent_completion_when_requested() -> None:
    manifest = {
        "benchmark_id": "unit-equivalent-completion-required",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag_local_first_final_land_packet"],
        "aggregate_candidate_mode": "ait_dag_local_first_final_land_packet",
        "minimum_comparable_long_workloads": 1,
        "require_equivalent_completion": True,
        "workloads": [
            {
                "workload_id": "long-demo",
                "category": "long",
                "runs": [
                    {
                        "run_id": "baseline",
                        "mode": "git_linear",
                        "usage_kind": "measured",
                        "quality": "passed",
                        "completion_status": "equivalent",
                        "usage": {"prompt_tokens": 1000, "completion_tokens": 500},
                    },
                    {
                        "run_id": "candidate",
                        "mode": "ait_dag_local_first_final_land_packet",
                        "usage_kind": "measured",
                        "quality": "passed",
                        "completion_status": "partial",
                        "usage": {"prompt_tokens": 100, "completion_tokens": 50},
                    },
                ],
            }
        ],
    }

    payload = evaluate_token_savings_manifest(manifest)

    assert payload["require_equivalent_completion"] is True
    assert payload["aggregate"]["claim_ready"] is False
    assert payload["aggregate"]["verdict"] == "unproven"
    comparison = payload["workloads"][0]["comparisons"][0]
    assert comparison["comparable"] is False
    assert comparison["candidate_run_count"] == 0
    assert comparison["candidate_median_total_tokens"] is None


def _write_codex_session_jsonl(path: Path) -> None:
    records = [
        {
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "last_token_usage": {
                        "input_tokens": 100,
                        "cached_input_tokens": 25,
                        "output_tokens": 20,
                        "reasoning_output_tokens": 5,
                        "total_tokens": 120,
                    },
                    "total_token_usage": {
                        "input_tokens": 100,
                        "cached_input_tokens": 25,
                        "output_tokens": 20,
                        "reasoning_output_tokens": 5,
                        "total_tokens": 120,
                    },
                },
            },
        },
        {
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "last_token_usage": {
                        "input_tokens": 30,
                        "cached_input_tokens": 10,
                        "output_tokens": 6,
                        "reasoning_output_tokens": 2,
                        "total_tokens": 36,
                    },
                    "total_token_usage": {
                        "input_tokens": 130,
                        "cached_input_tokens": 35,
                        "output_tokens": 26,
                        "reasoning_output_tokens": 7,
                        "total_tokens": 156,
                    },
                },
            },
        },
    ]
    path.write_text("\n".join(json.dumps(record) for record in records) + "\n", encoding="utf-8")


def _write_codex_turn_completed_jsonl(path: Path) -> None:
    records = [
        {"type": "thread.started", "thread_id": "thread_unit"},
        {"type": "turn.started", "turn_id": "turn_unit"},
        {
            "type": "turn.completed",
            "turn_id": "turn_unit",
            "usage": {
                "input_tokens": 210,
                "cached_input_tokens": 45,
                "output_tokens": 35,
                "reasoning_output_tokens": 9,
                "total_tokens": 245,
            },
        },
    ]
    path.write_text("\n".join(json.dumps(record) for record in records) + "\n", encoding="utf-8")


def test_extract_codex_token_usage_uses_latest_total_usage(tmp_path: Path) -> None:
    session_path = tmp_path / "codex-session.jsonl"
    _write_codex_session_jsonl(session_path)

    payload = extract_codex_token_usage(session_path)

    assert payload["token_event_count"] == 2
    assert payload["manifest_usage"]["prompt_tokens"] == 130
    assert payload["manifest_usage"]["completion_tokens"] == 26
    assert payload["manifest_usage"]["total_tokens"] == 156
    assert payload["manifest_usage"]["reasoning_output_tokens"] == 7


def test_extract_codex_token_usage_supports_turn_completed_usage(tmp_path: Path) -> None:
    session_path = tmp_path / "codex-turn-completed.jsonl"
    _write_codex_turn_completed_jsonl(session_path)

    payload = extract_codex_token_usage(session_path)

    assert payload["usage_source"] == "codex_exec_turn_completed_usage"
    assert payload["token_event_count"] == 1
    assert payload["manifest_usage"]["prompt_tokens"] == 210
    assert payload["manifest_usage"]["completion_tokens"] == 35
    assert payload["manifest_usage"]["total_tokens"] == 245
    assert payload["manifest_usage"]["cached_input_tokens"] == 45
    assert payload["manifest_usage"]["reasoning_output_tokens"] == 9


def test_extract_codex_token_usage_bundle_sums_multiple_sessions(tmp_path: Path) -> None:
    session_a = tmp_path / "codex-session-a.jsonl"
    session_b = tmp_path / "codex-session-b.jsonl"
    _write_codex_session_jsonl(session_a)
    _write_codex_session_jsonl(session_b)

    payload = extract_codex_token_usage_bundle([session_a, session_b])

    assert payload["session_count"] == 2
    assert payload["token_event_count"] == 4
    assert payload["manifest_usage"]["prompt_tokens"] == 260
    assert payload["manifest_usage"]["completion_tokens"] == 52
    assert payload["manifest_usage"]["total_tokens"] == 312
    assert payload["manifest_usage"]["reasoning_output_tokens"] == 14


def test_extract_codex_token_usage_bundle_tracks_role_breakdown(tmp_path: Path) -> None:
    session_a = tmp_path / "coordinator.jsonl"
    session_b = tmp_path / "worker.jsonl"
    _write_codex_session_jsonl(session_a)
    _write_codex_session_jsonl(session_b)

    payload = extract_codex_token_usage_bundle(
        [session_a, session_b],
        session_roles=["coordinator", "batch_worker"],
    )

    assert payload["role_breakdown"]["coordinator"]["usage"]["total_tokens"] == 156
    assert payload["role_breakdown"]["batch_worker"]["usage"]["total_tokens"] == 156
    assert payload["role_breakdown"]["coordinator"]["session_count"] == 1


def test_codex_fill_usage_cli_updates_manifest(tmp_path: Path) -> None:
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _write_codex_session_jsonl(Path("codex-session.jsonl"))
        manifest = {
            "benchmark_id": "codex-import",
            "baseline_mode": "git_linear",
            "candidate_modes": ["ait_dag"],
            "minimum_comparable_long_workloads": 1,
            "workloads": [
                {
                    "workload_id": "long-demo",
                    "category": "long",
                    "runs": [
                        {
                            "run_id": "baseline-run",
                            "mode": "git_linear",
                            "usage_kind": "measured",
                            "quality": "pending",
                            "usage": {"prompt_tokens": None, "completion_tokens": None},
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
                "codex-fill-usage",
                "--manifest",
                "manifest.json",
                "--run-session",
                "baseline-run=codex-session.jsonl",
                "--output-manifest",
                "filled.json",
                "--quality",
                "passed",
                "--json",
            ],
            catch_exceptions=False,
        )
        filled = json.loads(Path("filled.json").read_text(encoding="utf-8"))

    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["imported_count"] == 1
    run = filled["workloads"][0]["runs"][0]
    assert run["quality"] == "passed"
    assert run["usage_kind"] == "measured"
    assert run["usage"]["prompt_tokens"] == 130
    assert run["usage"]["completion_tokens"] == 26
    assert run["usage"]["total_tokens"] == 156


def test_codex_fill_usage_cli_aggregates_multiple_sessions_for_one_run(tmp_path: Path) -> None:
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _write_codex_session_jsonl(Path("codex-session-a.jsonl"))
        _write_codex_session_jsonl(Path("codex-session-b.jsonl"))
        manifest = {
            "benchmark_id": "codex-import-batch",
            "baseline_mode": "git_linear",
            "candidate_modes": ["remote_batch"],
            "aggregate_candidate_mode": "remote_batch",
            "minimum_comparable_long_workloads": 1,
            "workloads": [
                {
                    "workload_id": "solo-remote-batch",
                    "category": "long",
                    "runs": [
                        {
                            "run_id": "baseline-run",
                            "mode": "remote_batch",
                            "usage_kind": "measured",
                            "quality": "pending",
                            "usage": {"prompt_tokens": None, "completion_tokens": None},
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
                "codex-fill-usage",
                "--manifest",
                "manifest.json",
                "--run-session",
                "baseline-run=codex-session-a.jsonl",
                "--run-session",
                "baseline-run=codex-session-b.jsonl",
                "--output-manifest",
                "filled.json",
                "--quality",
                "passed",
                "--json",
            ],
            catch_exceptions=False,
        )
        filled = json.loads(Path("filled.json").read_text(encoding="utf-8"))

    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["imported_count"] == 1
    imported = payload["imported_runs"][0]
    assert imported["session_count"] == 2
    run = filled["workloads"][0]["runs"][0]
    assert run["quality"] == "passed"
    assert run["usage_kind"] == "measured"
    assert run["usage"]["prompt_tokens"] == 260
    assert run["usage"]["completion_tokens"] == 52
    assert run["usage"]["total_tokens"] == 312
    assert run["usage_provenance"]["session_count"] == 2


def test_codex_fill_usage_cli_records_role_breakdown(tmp_path: Path) -> None:
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _write_codex_session_jsonl(Path("coordinator.jsonl"))
        _write_codex_session_jsonl(Path("worker.jsonl"))
        manifest = {
            "benchmark_id": "codex-import-role-breakdown",
            "baseline_mode": "ait_dag",
            "candidate_modes": ["remote_batch"],
            "minimum_comparable_long_workloads": 1,
            "workloads": [
                {
                    "workload_id": "solo-remote-batch",
                    "category": "long",
                    "runs": [
                        {
                            "run_id": "batch-run",
                            "mode": "remote_batch",
                            "usage_kind": "measured",
                            "quality": "pending",
                            "usage": {"prompt_tokens": None, "completion_tokens": None},
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
                "codex-fill-usage",
                "--manifest",
                "manifest.json",
                "--run-role-session",
                "batch-run:coordinator=coordinator.jsonl",
                "--run-role-session",
                "batch-run:batch_worker=worker.jsonl",
                "--output-manifest",
                "filled.json",
                "--quality",
                "passed",
                "--json",
            ],
            catch_exceptions=False,
        )
        filled = json.loads(Path("filled.json").read_text(encoding="utf-8"))

    assert result.exit_code == 0, result.stdout
    run = filled["workloads"][0]["runs"][0]
    assert run["usage_breakdown"]["coordinator"]["total_tokens"] == 156
    assert run["usage_breakdown"]["batch_worker"]["total_tokens"] == 156
    assert run["usage_provenance"]["role_breakdown"]["coordinator"]["session_count"] == 1


def test_token_savings_benchmark_supports_comparison_profiles_and_cost_accounting() -> None:
    manifest = {
        "benchmark_id": "unit-remote-batch-profiled",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_linear", "ait_dag"],
        "aggregate_candidate_mode": "ait_dag",
        "minimum_comparable_long_workloads": 1,
        "comparison_profiles": [
            {
                "profile_id": "packet_claim_baseline",
                "title": "Packet-only claim baseline",
                "baseline_mode": "git_linear",
                "candidate_modes": ["ait_linear", "ait_dag"],
                "aggregate_candidate_mode": "ait_dag",
                "claim_target": True,
            },
            {
                "profile_id": "remote_orchestration",
                "title": "Remote orchestration cost",
                "baseline_mode": "ait_dag",
                "candidate_modes": ["remote_batch"],
                "aggregate_candidate_mode": "remote_batch",
                "claim_target": False,
            },
        ],
        "workloads": [
            {
                "workload_id": "long-demo",
                "category": "long",
                "runs": [
                    {
                        "run_id": "baseline",
                        "mode": "git_linear",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 1000, "completion_tokens": 500},
                    },
                    {
                        "run_id": "linear",
                        "mode": "ait_linear",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 850, "completion_tokens": 450},
                    },
                    {
                        "run_id": "dag",
                        "mode": "ait_dag",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 600, "completion_tokens": 300},
                    },
                    {
                        "run_id": "batch",
                        "mode": "remote_batch",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 900, "completion_tokens": 450},
                        "usage_breakdown": {
                            "coordinator": {"session_count": 1, "prompt_tokens": 400, "completion_tokens": 200, "total_tokens": 600},
                            "batch_worker": {"session_count": 2, "prompt_tokens": 500, "completion_tokens": 250, "total_tokens": 750},
                        },
                    },
                ],
            }
        ],
    }

    payload = evaluate_token_savings_manifest(manifest)
    report = render_token_savings_markdown(payload)

    assert payload["aggregate"]["aggregate_candidate_mode"] == "ait_dag"
    assert payload["profiles"][0]["aggregate"]["claim_ready"] is True
    assert payload["profiles"][1]["aggregate"]["claim_target"] is False
    assert payload["profiles"][1]["aggregate"]["claim_caveat"].startswith("This profile measures real orchestration cost")
    assert "## Cost accounting" in report
    assert "coordinator" in report


def test_token_savings_benchmark_supports_server_warm_profile_filters() -> None:
    manifest = {
        "benchmark_id": "unit-remote-batch-server-warm",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag"],
        "aggregate_candidate_mode": "ait_dag",
        "minimum_comparable_long_workloads": 1,
        "comparison_profiles": [
            {
                "profile_id": "packet_claim_baseline",
                "title": "Packet-only claim baseline",
                "baseline_mode": "git_linear",
                "candidate_modes": ["ait_dag"],
                "aggregate_candidate_mode": "ait_dag",
                "claim_target": True,
                "workload_tags": ["standard"],
            },
            {
                "profile_id": "server_warm_resumed",
                "title": "Server-warm repeated or resumed profile",
                "profile_kind": "server_warm_resumed",
                "baseline_mode": "ait_dag",
                "candidate_modes": ["remote_batch"],
                "aggregate_candidate_mode": "remote_batch",
                "workload_tags": ["server_warm", "resumed"],
            },
        ],
        "workloads": [
            {
                "workload_id": "fresh-demo",
                "category": "long",
                "tags": ["standard"],
                "runs": [
                    {
                        "run_id": "fresh-baseline",
                        "mode": "git_linear",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 1000, "completion_tokens": 500},
                    },
                    {
                        "run_id": "fresh-dag",
                        "mode": "ait_dag",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 700, "completion_tokens": 300},
                    },
                ],
            },
            {
                "workload_id": "server-warm-demo",
                "category": "long",
                "tags": ["server_warm", "resumed"],
                "runs": [
                    {
                        "run_id": "warm-baseline",
                        "mode": "ait_dag",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 700, "completion_tokens": 300},
                    },
                    {
                        "run_id": "warm-candidate",
                        "mode": "remote_batch",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 500, "completion_tokens": 250},
                    },
                ],
            },
        ],
    }

    payload = evaluate_token_savings_manifest(manifest)
    report = render_token_savings_markdown(payload)

    assert payload["profiles"][0]["workloads"][0]["workload_id"] == "fresh-demo"
    assert payload["profiles"][1]["profile_kind"] == "server_warm_resumed"
    assert payload["profiles"][1]["experimental"] is True
    assert payload["profiles"][1]["aggregate"]["claim_target"] is False
    assert [row["workload_id"] for row in payload["profiles"][1]["workloads"]] == ["server-warm-demo"]
    assert "Profile kind: `server_warm_resumed`" in report
    assert "Workload tags: `resumed, server_warm`" in report


def test_token_savings_benchmark_supports_same_surface_review_overhead_profiles() -> None:
    manifest = {
        "benchmark_id": "unit-dual-review-overhead",
        "baseline_mode": "ait_dag",
        "candidate_modes": ["ait_dag_dual_review"],
        "aggregate_candidate_mode": "ait_dag_dual_review",
        "minimum_comparable_long_workloads": 1,
        "comparison_profiles": [
            {
                "profile_id": "dual_review_overhead",
                "title": "Same-surface dual-review overhead",
                "profile_kind": "same_surface_review_overhead",
                "experimental": True,
                "baseline_mode": "ait_dag",
                "candidate_modes": ["ait_dag_dual_review"],
                "aggregate_candidate_mode": "ait_dag_dual_review",
                "minimum_comparable_long_workloads": 1,
                "claim_target": False,
                "workload_ids": ["task_review_default_mode_handoff"],
            }
        ],
        "workloads": [
            {
                "workload_id": "task_review_default_mode_handoff",
                "category": "long",
                "runs": [
                    {
                        "run_id": "baseline",
                        "mode": "ait_dag",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 1000, "completion_tokens": 500},
                    },
                    {
                        "run_id": "candidate",
                        "mode": "ait_dag_dual_review",
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 1000, "completion_tokens": 550},
                    },
                ],
            }
        ],
    }

    payload = evaluate_token_savings_manifest(manifest)
    report = render_token_savings_markdown(payload)

    assert payload["profiles"][0]["aggregate"]["claim_target"] is False
    assert payload["profiles"][0]["aggregate"]["claim_caveat"].startswith("Same-surface packet-overhead evidence only")
    assert "same-surface packet overhead only" in report.lower()


def test_build_strict_rerun_fixture_bundle_prepares_git_and_ait_fixtures(tmp_path: Path) -> None:
    source_root = tmp_path / "source"
    source_root.mkdir()
    (source_root / "README.md").write_text("hello\n", encoding="utf-8")
    (source_root / "notes.txt").write_text("benchmark fixture\n", encoding="utf-8")
    (source_root / ".ait").mkdir()
    (source_root / ".ait" / "should-not-copy.txt").write_text("ignore me\n", encoding="utf-8")

    payload = build_strict_rerun_fixture_bundle(
        benchmark_id="strict-rerun-unit",
        output_dir=source_root / ".ait" / "generated" / "strict-rerun-unit",
        source_root=source_root,
        source_snapshot_id="SNP-UNIT-1234",
        workloads=[
            {
                "workload_id": "long-demo",
                "title": "Long demo",
                "category": "long",
                "acceptance": "A strict rerun fixture exists.",
            }
        ],
    )

    assert payload["fixture_count"] == 3
    manifest_path = Path(payload["manifest_path"])
    fixture_bundle_path = Path(payload["fixture_bundle_path"])
    readme_path = Path(payload["readme_path"])
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    fixture_bundle = json.loads(fixture_bundle_path.read_text(encoding="utf-8"))
    assert manifest["strict_rerun_protocol"]["source_snapshot_id"] == "SNP-UNIT-1234"
    assert manifest["candidate_modes"] == ["ait_linear", "ait_dag"]
    assert manifest["require_equivalent_completion"] is True
    assert manifest["workloads"][0]["runs"][0]["completion_status"] == "pending"
    assert readme_path.exists()
    readme = readme_path.read_text(encoding="utf-8")
    assert "codex-fill-usage" in readme
    assert "completion_status" in readme

    git_fixture = fixture_bundle["fixtures"][0]
    git_fixture_root = Path(git_fixture["fixture_root"])
    assert git_fixture["mode"] == "git_linear"
    assert (git_fixture_root / "README.md").exists()
    assert not (git_fixture_root / ".ait").exists()
    assert git_fixture["git"]["clean_status_before_measured_run"] == ""

    ait_modes = {row["mode"]: row for row in fixture_bundle["fixtures"][1:]}
    assert set(ait_modes.keys()) == {"ait_linear", "ait_dag"}
    for row in ait_modes.values():
        fixture_root = Path(row["fixture_root"])
        assert (fixture_root / ".ait" / "config.json").exists()
        assert row["git"]["prepared_head_commit_id"] != row["git"]["initial_commit_id"]
        assert row["git"]["clean_status_before_measured_run"] == ""


def test_build_strict_rerun_fixture_bundle_supports_local_first_final_land_packet_mode(tmp_path: Path) -> None:
    source_root = tmp_path / "source"
    source_root.mkdir()
    (source_root / "README.md").write_text("hello\n", encoding="utf-8")

    payload = build_strict_rerun_fixture_bundle(
        benchmark_id="strict-rerun-local-first",
        output_dir=source_root / ".ait" / "generated" / "strict-rerun-local-first",
        source_root=source_root,
        workloads=[
            {
                "workload_id": "long-demo",
                "title": "Long demo",
                "category": "long",
                "acceptance": "Prepared",
            }
        ],
        candidate_modes=["ait_linear", "ait_dag_local_first_final_land_packet"],
        aggregate_candidate_mode="ait_dag_local_first_final_land_packet",
    )

    manifest = json.loads(Path(payload["manifest_path"]).read_text(encoding="utf-8"))

    assert manifest["candidate_modes"] == ["ait_linear", "ait_dag_local_first_final_land_packet"]
    assert manifest["aggregate_candidate_mode"] == "ait_dag_local_first_final_land_packet"
    packet_run = next(
        run
        for run in manifest["workloads"][0]["runs"]
        if run["mode"] == "ait_dag_local_first_final_land_packet"
    )
    assert packet_run["completion_status"] == "pending"
    assert "equivalent completion" in packet_run["notes"].lower()


def test_benchmark_strict_rerun_cli_builds_pending_manifest_from_seed(tmp_path: Path) -> None:
    with runner.isolated_filesystem(temp_dir=tmp_path):
        Path("source").mkdir()
        Path("source/README.md").write_text("hello\n", encoding="utf-8")
        seed_manifest = {
            "benchmark_id": "seed",
            "baseline_mode": "git_linear",
            "candidate_modes": ["ait_linear", "ait_dag"],
            "workloads": [
                {
                    "workload_id": "seed-workload",
                    "title": "Seed workload",
                    "category": "long",
                    "acceptance": "Prepared",
                    "runs": [],
                }
            ],
        }
        Path("seed.json").write_text(json.dumps(seed_manifest), encoding="utf-8")

        result = runner.invoke(
            app,
            [
                "benchmark",
                "strict-rerun",
                "--benchmark-id",
                "strict-cli-unit",
                "--source-root",
                "source",
                "--source-snapshot-id",
                "SNP-CLI-0001",
                "--seed-manifest",
                "seed.json",
                "--output-dir",
                "bundle",
                "--json",
            ],
            catch_exceptions=False,
        )

        assert result.exit_code == 0, result.stdout
        payload = json.loads(result.stdout)
        assert payload["benchmark_id"] == "strict-cli-unit"
        manifest = json.loads(Path(payload["manifest_path"]).read_text(encoding="utf-8"))
        assert manifest["workloads"][0]["workload_id"] == "seed-workload"
        assert manifest["workloads"][0]["runs"][0]["quality"] == "pending"
        assert manifest["workloads"][0]["runs"][0]["usage"]["total_tokens"] is None
        assert Path(payload["readme_path"]).exists()
