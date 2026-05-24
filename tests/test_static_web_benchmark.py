from __future__ import annotations

import json
from pathlib import Path

from typer.testing import CliRunner

from ait.cli import app
from ait.static_web_benchmark import (
    evaluate_static_web_task_manifest,
    render_static_web_task_markdown,
)


runner = CliRunner()


def test_static_web_benchmark_supports_comparable_candidate_run() -> None:
    manifest = {
        "benchmark_id": "unit-static-web-task",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag_current"],
        "aggregate_candidate_mode": "ait_dag_current",
        "minimum_comparable_runs": 1,
        "workloads": [
            {
                "workload_id": "plane-shooter",
                "category": "medium",
                "comparison_style": "reviewed_plan",
                "runs": [
                    {
                        "run_id": "baseline-1",
                        "mode": "git_linear",
                        "startup_script": "start.sh",
                        "startup_check_passed": True,
                        "fixture_root": "fixtures/git-linear",
                        "entry_path": "index.html",
                        "entry_url": "http://127.0.0.1:34567/index.html",
                        "benchmark_url": "http://127.0.0.1:34567/index.html?seed=1337&bossAfter=10",
                        "equivalent_task_completion": True,
                        "required_behavior": {
                            "player_controls": True,
                            "enemy_spawns": True,
                            "game_over": True,
                            "victory": True,
                        },
                        "harness": {
                            "seed_control_present": True,
                            "fast_boss_trigger_present": True,
                            "helper_controls_documented": True,
                            "gameplay_rules_identical": True,
                        },
                        "rubric": {
                            "functional_completeness": 44,
                            "deterministic_benchmark_harness": 15,
                            "style_fidelity": 12,
                            "code_structure_and_integration_quality": 13,
                            "benchmark_hygiene": 9,
                        },
                        "elapsed_seconds": 420,
                        "commands_run": 20,
                        "files_read": 12,
                        "files_edited": 5,
                        "tests_or_manual_checks": ["startup script", "seed=1337 bossAfter=10 validation"],
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 700, "completion_tokens": 300},
                        "task_review_summary": "baseline completed the requested workload",
                        "code_review_summary": "baseline implementation is serviceable",
                    },
                    {
                        "run_id": "candidate-1",
                        "mode": "ait_dag_current",
                        "startup_script": "start.sh",
                        "startup_check_passed": True,
                        "fixture_root": "fixtures/ait-dag-current",
                        "entry_path": "index.html",
                        "entry_url": "http://127.0.0.1:34567/index.html",
                        "benchmark_url": "http://127.0.0.1:34567/index.html?seed=1337&bossAfter=10",
                        "equivalent_task_completion": True,
                        "required_behavior": {
                            "player_controls": True,
                            "enemy_spawns": True,
                            "game_over": True,
                            "victory": True,
                        },
                        "harness": {
                            "seed_control_present": True,
                            "fast_boss_trigger_present": True,
                            "helper_controls_documented": True,
                            "gameplay_rules_identical": True,
                        },
                        "rubric": {
                            "functional_completeness": 45,
                            "deterministic_benchmark_harness": 15,
                            "style_fidelity": 13,
                            "code_structure_and_integration_quality": 14,
                            "benchmark_hygiene": 10,
                        },
                        "elapsed_seconds": 360,
                        "commands_run": 18,
                        "files_read": 10,
                        "files_edited": 5,
                        "tests_or_manual_checks": ["startup script", "boss flow", "victory state"],
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 680, "completion_tokens": 290},
                        "task_review_summary": "candidate completed the requested workload",
                        "code_review_summary": "candidate stayed modular and benchmark-friendly",
                    },
                ],
            }
        ],
    }

    payload = evaluate_static_web_task_manifest(manifest)

    assert payload["evidence_type"] == "measured"
    assert payload["aggregate"]["verdict"] == "supported"
    assert payload["aggregate"]["candidate_comparable_run_count"] == 1
    assert payload["aggregate"]["candidate_median_score"] == 97
    report = render_static_web_task_markdown(payload)
    assert "unit-static-web-task" in report
    assert "ait_dag_current" in report
    assert "start.sh" in report


def test_static_web_benchmark_marks_startup_failure_as_blocker() -> None:
    manifest = {
        "benchmark_id": "unit-static-web-task-blocker",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag_current"],
        "aggregate_candidate_mode": "ait_dag_current",
        "minimum_comparable_runs": 1,
        "workloads": [
            {
                "workload_id": "plane-shooter",
                "category": "medium",
                "runs": [
                    {
                        "run_id": "baseline-1",
                        "mode": "git_linear",
                        "startup_script": "start.sh",
                        "startup_check_passed": True,
                        "equivalent_task_completion": True,
                        "required_behavior": {"game_over": True, "victory": True},
                        "harness": {"seed_control_present": True, "fast_boss_trigger_present": True},
                        "rubric": {
                            "functional_completeness": 40,
                            "deterministic_benchmark_harness": 15,
                            "style_fidelity": 10,
                            "code_structure_and_integration_quality": 10,
                            "benchmark_hygiene": 8,
                        },
                    },
                    {
                        "run_id": "candidate-1",
                        "mode": "ait_dag_current",
                        "startup_script": "",
                        "startup_check_passed": False,
                        "equivalent_task_completion": True,
                        "required_behavior": {"game_over": True, "victory": True},
                        "harness": {"seed_control_present": True, "fast_boss_trigger_present": True},
                        "rubric": {
                            "functional_completeness": 45,
                            "deterministic_benchmark_harness": 15,
                            "style_fidelity": 13,
                            "code_structure_and_integration_quality": 14,
                            "benchmark_hygiene": 10,
                        },
                    },
                ],
            }
        ],
    }

    payload = evaluate_static_web_task_manifest(manifest)
    run = payload["workloads"][0]["runs"][1]

    assert run["pass_or_fail"] == "fail"
    assert run["comparable"] is False
    assert "Missing project-local startup script." in run["blocking_findings"]
    assert payload["aggregate"]["verdict"] == "not_supported"


def test_benchmark_static_web_task_cli_writes_reports(tmp_path: Path) -> None:
    with runner.isolated_filesystem(temp_dir=tmp_path):
        manifest = {
            "benchmark_id": "unit-static-web-cli",
            "baseline_mode": "git_linear",
            "candidate_modes": ["ait_dag_current"],
            "aggregate_candidate_mode": "ait_dag_current",
            "minimum_comparable_runs": 1,
            "workloads": [
                {
                    "workload_id": "plane-shooter",
                    "category": "medium",
                    "runs": [
                        {
                            "run_id": "baseline-1",
                            "mode": "git_linear",
                            "startup_script": "start.sh",
                            "startup_check_passed": True,
                            "equivalent_task_completion": True,
                            "required_behavior": {"game_over": True, "victory": True},
                            "harness": {"seed_control_present": True, "fast_boss_trigger_present": True},
                            "rubric": {
                                "functional_completeness": 41,
                                "deterministic_benchmark_harness": 15,
                                "style_fidelity": 11,
                                "code_structure_and_integration_quality": 12,
                                "benchmark_hygiene": 8,
                            },
                            "usage_kind": "measured",
                            "usage": {"prompt_tokens": 610, "completion_tokens": 250},
                        },
                        {
                            "run_id": "candidate-1",
                            "mode": "ait_dag_current",
                            "startup_script": "start.sh",
                            "startup_check_passed": True,
                            "equivalent_task_completion": True,
                            "required_behavior": {"game_over": True, "victory": True},
                            "harness": {"seed_control_present": True, "fast_boss_trigger_present": True},
                            "rubric": {
                                "functional_completeness": 43,
                                "deterministic_benchmark_harness": 15,
                                "style_fidelity": 12,
                                "code_structure_and_integration_quality": 13,
                                "benchmark_hygiene": 9,
                            },
                            "usage_kind": "measured",
                            "usage": {"prompt_tokens": 590, "completion_tokens": 240},
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
                "static-web-task",
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
        assert "unit-static-web-cli" in Path("out/report.md").read_text(encoding="utf-8")
