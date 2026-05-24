from __future__ import annotations

import json
from pathlib import Path

from typer.testing import CliRunner

from ait.cli import app
from ait.static_web_hardening_benchmark import (
    compute_fixture_digest,
    evaluate_static_web_hardening_task_manifest,
    render_static_web_hardening_task_markdown,
    run_static_web_hardening_task_benchmark,
)


runner = CliRunner()
SHARED_TASK_DOCS = [
    "docs/benchmark_task.md",
    "docs/reviewed_execution_plan.md",
    "docs/acceptance.md",
    "docs/ait_linear_plan.md",
]
CONTRACT_FILES = [
    "docs/replay_schema.md",
    "docs/settings_schema.md",
    "docs/benchmark_runbook.md",
    "docs/release_checklist.md",
]


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


def _scaffold_hardening_fixture(root: Path) -> None:
    (root / "docs").mkdir(parents=True, exist_ok=True)
    (root / "index.html").write_text("<!doctype html><title>fixture</title>", encoding="utf-8")
    (root / "start.sh").write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
    (root / "validate.sh").write_text("#!/usr/bin/env bash\nexit 0\n", encoding="utf-8")
    (root / "docs" / "benchmark_task.md").write_text("# benchmark task\n", encoding="utf-8")
    (root / "docs" / "reviewed_execution_plan.md").write_text("# plan\n", encoding="utf-8")
    (root / "docs" / "acceptance.md").write_text("# acceptance\n", encoding="utf-8")
    (root / "docs" / "ait_linear_plan.md").write_text("# ait linear scaffold\n", encoding="utf-8")
    (root / "docs" / "replay_schema.md").write_text("# replay\n", encoding="utf-8")
    (root / "docs" / "settings_schema.md").write_text("# settings\n", encoding="utf-8")
    (root / "docs" / "benchmark_runbook.md").write_text("# benchmark\n", encoding="utf-8")
    (root / "docs" / "release_checklist.md").write_text("# release\n", encoding="utf-8")


def test_static_web_hardening_benchmark_supports_comparable_candidate_run(tmp_path: Path) -> None:
    baseline_root = tmp_path / "baseline"
    git_root = tmp_path / "git-linear"
    dag_root = tmp_path / "ait-dag-current"
    for root in (baseline_root, git_root, dag_root):
        _scaffold_hardening_fixture(root)

    manifest = {
        "benchmark_id": "unit-static-web-hardening",
        "comparison_family": "core_single_session",
        "bootstrap_surface": "reality",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag_current"],
        "aggregate_candidate_mode": "ait_dag_current",
        "minimum_comparable_runs": 1,
        "baseline_fixture": {
            "fixture_id": "baseline-shooter",
            "fixture_root": str(baseline_root),
            "snapshot_id": "BASELINE-SNAPSHOT-20260508",
            "digest": compute_fixture_digest(baseline_root),
            "source_artifact": "docs/benchmarks/fixtures/static_web_plane_shooter_medium_baseline.fixture.json",
            "shared_task_docs": SHARED_TASK_DOCS,
        },
        "workloads": [
            {
                "workload_id": "plane-shooter-hardening",
                "category": "long",
                "comparison_style": "reviewed_plan",
                "runs": [
                    {
                        "run_id": "baseline-1",
                        "mode": "git_linear",
                        "fixture_root": str(git_root),
                        "entry_path": "index.html",
                        "entry_url": "http://127.0.0.1:4473/index.html",
                        "benchmark_url": "http://127.0.0.1:4473/index.html?seed=1337&bossAfter=10",
                        "startup_script": "start.sh",
                        "validation_script": "validate.sh",
                        "startup_check_passed": True,
                        "validation_check_passed": True,
                        "evaluator_runtime_closeout_status": "passed",
                        "topology_id": "linear-single-session",
                        "measured_session_count": 1,
                        "provider_usage_source": "codex session jsonl",
                        "equivalent_task_completion": True,
                        "surface_reproducible": True,
                        "medium_regression_passed": True,
                        "required_behavior": {
                            "player_controls": True,
                            "enemy_spawns": True,
                            "game_over": True,
                            "victory": True,
                        },
                        "replay_check_status": "passed",
                        "settings_check_status": "passed",
                        "mobile_input_check_status": "passed",
                        "shared_task_docs": SHARED_TASK_DOCS,
                        "contract_files": CONTRACT_FILES,
                        "rubric": {
                            "regression_free_medium_behavior": 19,
                            "hardening_systems_completeness": 18,
                            "shared_contracts_and_release_docs": 18,
                            "validation_and_startup_quality": 14,
                            "code_structure_and_convergence_quality": 13,
                            "benchmark_hygiene": 9,
                        },
                        "elapsed_seconds": 620,
                        "commands_run": 28,
                        "files_read": 18,
                        "files_edited": 9,
                        "tests_or_manual_checks": ["startup", "validation", "replay", "mobile"],
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 1200, "completion_tokens": 410},
                        "task_review_summary": "baseline completed the hardening workload",
                        "code_review_summary": "baseline remained coherent",
                        "release_readiness_summary": "startup and validation passed",
                    },
                    {
                        "run_id": "candidate-1",
                        "mode": "ait_dag_current",
                        "fixture_root": str(dag_root),
                        "entry_path": "index.html",
                        "entry_url": "http://127.0.0.1:4573/index.html",
                        "benchmark_url": "http://127.0.0.1:4573/index.html?seed=1337&bossAfter=10",
                        "startup_script": "start.sh",
                        "validation_script": "validate.sh",
                        "startup_check_passed": True,
                        "validation_check_passed": True,
                        "evaluator_runtime_closeout_status": "passed",
                        "topology_id": "full-dag-json-one-worker",
                        "measured_session_count": 1,
                        "dag_cost_accounting_policy": "reported_separately",
                        "dag_cost_breakdown": {
                            "sprint_card_setup": {
                                "session_count": 1,
                                "prompt_tokens": 120,
                                "completion_tokens": 40,
                                "total_tokens": 160,
                            },
                            "dag_json_authoring": {
                                "session_count": 1,
                                "prompt_tokens": 180,
                                "completion_tokens": 60,
                                "total_tokens": 240,
                            },
                            "worker_execution": {
                                "session_count": 1,
                                "prompt_tokens": 980,
                                "completion_tokens": 360,
                                "total_tokens": 1340,
                            },
                        },
                        "provider_usage_source": "codex session jsonl",
                        "equivalent_task_completion": True,
                        "surface_reproducible": True,
                        "medium_regression_passed": True,
                        "required_behavior": {
                            "player_controls": True,
                            "enemy_spawns": True,
                            "game_over": True,
                            "victory": True,
                        },
                        "replay_check_status": "passed",
                        "settings_check_status": "passed",
                        "mobile_input_check_status": "passed",
                        "shared_task_docs": SHARED_TASK_DOCS,
                        "contract_files": CONTRACT_FILES,
                        "rubric": {
                            "regression_free_medium_behavior": 20,
                            "hardening_systems_completeness": 19,
                            "shared_contracts_and_release_docs": 19,
                            "validation_and_startup_quality": 15,
                            "code_structure_and_convergence_quality": 14,
                            "benchmark_hygiene": 10,
                        },
                        "elapsed_seconds": 540,
                        "commands_run": 22,
                        "files_read": 15,
                        "files_edited": 9,
                        "tests_or_manual_checks": ["startup", "validation", "replay", "mobile"],
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 980, "completion_tokens": 360},
                        "task_review_summary": "candidate completed the hardening workload",
                        "code_review_summary": "candidate stayed converged",
                        "release_readiness_summary": "all required validation surfaces passed",
                    },
                ],
            }
        ],
    }

    payload = evaluate_static_web_hardening_task_manifest(manifest)

    assert payload["evidence_type"] == "measured"
    assert payload["aggregate"]["verdict"] == "supported"
    assert payload["aggregate"]["candidate_comparable_run_count"] == 1
    assert payload["baseline_mode_label"] == "git_linear"
    assert payload["aggregate_candidate_mode_label"] == "ait_dag_current"
    assert payload["workloads"][0]["runs"][1]["dag_preparation_usage"]["total_tokens"] == 400
    assert payload["workloads"][0]["runs"][1]["dag_execution_usage"]["total_tokens"] == 1340
    assert payload["workloads"][0]["runs"][1]["baseline_fixture_digest"] == compute_fixture_digest(baseline_root)
    report = render_static_web_hardening_task_markdown(payload)
    assert "unit-static-web-hardening" in report
    assert "full-dag-json-one-worker" in report
    assert "## Cost Accounting" in report
    assert "sprint_card_setup" in report
    assert "dag_json_authoring" in report
    assert "validate.sh" in report


def test_static_web_hardening_benchmark_marks_validation_and_contract_failures(tmp_path: Path) -> None:
    baseline_root = tmp_path / "baseline"
    git_root = tmp_path / "git-linear"
    dag_root = tmp_path / "ait-dag-current"
    for root in (baseline_root, git_root, dag_root):
        _scaffold_hardening_fixture(root)

    manifest = {
        "benchmark_id": "unit-static-web-hardening-blocker",
        "comparison_family": "core_single_session",
        "bootstrap_surface": "reality",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag_current"],
        "aggregate_candidate_mode": "ait_dag_current",
        "minimum_comparable_runs": 1,
        "baseline_fixture": {
            "fixture_root": str(baseline_root),
            "snapshot_id": "BASELINE-SNAPSHOT-20260508",
            "digest": compute_fixture_digest(baseline_root),
            "shared_task_docs": SHARED_TASK_DOCS,
        },
        "workloads": [
            {
                "workload_id": "plane-shooter-hardening",
                "category": "long",
                "runs": [
                    {
                        "run_id": "baseline-1",
                        "mode": "git_linear",
                        "fixture_root": str(git_root),
                        "entry_path": "index.html",
                        "entry_url": "http://127.0.0.1:4473/index.html",
                        "benchmark_url": "http://127.0.0.1:4473/index.html?seed=1337&bossAfter=10",
                        "startup_script": "start.sh",
                        "validation_script": "validate.sh",
                        "startup_check_passed": True,
                        "validation_check_passed": True,
                        "evaluator_runtime_closeout_status": "passed",
                        "equivalent_task_completion": True,
                        "surface_reproducible": True,
                        "medium_regression_passed": True,
                        "required_behavior": {"game_over": True, "victory": True},
                        "replay_check_status": "passed",
                        "settings_check_status": "passed",
                        "mobile_input_check_status": "passed",
                        "shared_task_docs": SHARED_TASK_DOCS,
                        "contract_files": CONTRACT_FILES,
                        "rubric": {
                            "regression_free_medium_behavior": 18,
                            "hardening_systems_completeness": 18,
                            "shared_contracts_and_release_docs": 18,
                            "validation_and_startup_quality": 13,
                            "code_structure_and_convergence_quality": 13,
                            "benchmark_hygiene": 9,
                        },
                    },
                    {
                        "run_id": "candidate-1",
                        "mode": "ait_dag_current",
                        "fixture_root": str(dag_root),
                        "entry_path": "index.html",
                        "entry_url": "http://127.0.0.1:4573/index.html",
                        "benchmark_url": "http://127.0.0.1:4573/index.html?seed=1337&bossAfter=10",
                        "startup_script": "start.sh",
                        "validation_script": "",
                        "startup_check_passed": True,
                        "validation_check_passed": False,
                        "evaluator_runtime_closeout_status": "passed",
                        "equivalent_task_completion": True,
                        "surface_reproducible": True,
                        "medium_regression_passed": True,
                        "required_behavior": {"game_over": True, "victory": True},
                        "replay_check_status": "passed",
                        "settings_check_status": "not_run",
                        "mobile_input_check_status": "failed",
                        "shared_task_docs": SHARED_TASK_DOCS,
                        "contract_files": ["docs/replay_schema.md"],
                        "rubric": {
                            "regression_free_medium_behavior": 20,
                            "hardening_systems_completeness": 17,
                            "shared_contracts_and_release_docs": 10,
                            "validation_and_startup_quality": 8,
                            "code_structure_and_convergence_quality": 13,
                            "benchmark_hygiene": 9,
                        },
                    },
                ],
            }
        ],
    }

    payload = evaluate_static_web_hardening_task_manifest(manifest)
    run = payload["workloads"][0]["runs"][1]

    assert run["pass_or_fail"] == "fail"
    assert run["comparable"] is False
    assert "Missing project-local validation script." in run["blocking_findings"]
    assert "Settings validation did not pass." in run["blocking_findings"]
    assert any("Missing contract artifact families" in item for item in run["blocking_findings"])
    assert payload["aggregate"]["verdict"] == "not_supported"


def test_static_web_hardening_benchmark_distinguishes_reality_and_normalized_ait_linear_surfaces(
    tmp_path: Path,
) -> None:
    baseline_root = tmp_path / "baseline"
    git_root = tmp_path / "git-linear"
    linear_root = tmp_path / "ait-linear"
    for root in (baseline_root, git_root, linear_root):
        _scaffold_hardening_fixture(root)

    baseline_run = {
        "run_id": "baseline-1",
        "mode": "git_linear",
        "fixture_root": str(git_root),
        "entry_path": "index.html",
        "entry_url": "http://127.0.0.1:4473/index.html",
        "benchmark_url": "http://127.0.0.1:4473/index.html?seed=1337&bossAfter=10",
        "startup_script": "start.sh",
        "validation_script": "validate.sh",
        "startup_check_passed": True,
        "validation_check_passed": True,
        "evaluator_runtime_closeout_status": "passed",
        "topology_id": "linear-single-session",
        "measured_session_count": 1,
        "provider_usage_source": "codex session jsonl",
        "equivalent_task_completion": True,
        "surface_reproducible": True,
        "medium_regression_passed": True,
        "required_behavior": {"game_over": True, "victory": True},
        "replay_check_status": "passed",
        "settings_check_status": "passed",
        "mobile_input_check_status": "passed",
        "shared_task_docs": SHARED_TASK_DOCS,
        "contract_files": CONTRACT_FILES,
        "rubric": {
            "regression_free_medium_behavior": 18,
            "hardening_systems_completeness": 18,
            "shared_contracts_and_release_docs": 18,
            "validation_and_startup_quality": 14,
            "code_structure_and_convergence_quality": 13,
            "benchmark_hygiene": 9,
        },
        "usage_kind": "measured",
        "usage": {"prompt_tokens": 900, "completion_tokens": 320},
    }
    base_manifest = {
        "benchmark_id": "unit-static-web-hardening-ait-linear-surfaces",
        "comparison_family": "core_single_session",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_linear"],
        "aggregate_candidate_mode": "ait_linear",
        "minimum_comparable_runs": 1,
        "baseline_fixture": {
            "fixture_root": str(baseline_root),
            "snapshot_id": "BASELINE-SNAPSHOT-20260508",
            "digest": compute_fixture_digest(baseline_root),
            "shared_task_docs": SHARED_TASK_DOCS,
        },
        "workloads": [
            {
                "workload_id": "plane-shooter-hardening",
                "category": "long",
                "runs": [baseline_run],
            }
        ],
    }

    reality_manifest = json.loads(json.dumps(base_manifest))
    reality_manifest["bootstrap_surface"] = "reality"
    reality_manifest["workloads"][0]["runs"].append(
        {
            "run_id": "candidate-reality",
            "mode": "ait_linear",
            "fixture_root": str(linear_root),
            "entry_path": "index.html",
            "entry_url": "http://127.0.0.1:4573/index.html",
            "benchmark_url": "http://127.0.0.1:4573/index.html?seed=1337&bossAfter=10",
            "startup_script": "start.sh",
            "validation_script": "validate.sh",
            "startup_check_passed": True,
            "validation_check_passed": True,
            "evaluator_runtime_closeout_status": "passed",
            "topology_id": "ait-linear-single-session",
            "measured_session_count": 1,
            "provider_usage_source": "codex session jsonl",
            "equivalent_task_completion": True,
            "surface_reproducible": True,
            "medium_regression_passed": True,
            "required_behavior": {"game_over": True, "victory": True},
            "replay_check_status": "passed",
            "settings_check_status": "passed",
            "mobile_input_check_status": "passed",
            "shared_task_docs": SHARED_TASK_DOCS,
            "ait_linear_binding_path": "advisory_override",
            "plan_task_binding_mode": "advisory",
            "contract_files": CONTRACT_FILES,
            "rubric": {
                "regression_free_medium_behavior": 19,
                "hardening_systems_completeness": 18,
                "shared_contracts_and_release_docs": 18,
                "validation_and_startup_quality": 14,
                "code_structure_and_convergence_quality": 13,
                "benchmark_hygiene": 9,
            },
            "usage_kind": "measured",
            "usage": {"prompt_tokens": 820, "completion_tokens": 300},
        }
    )

    reality_payload = evaluate_static_web_hardening_task_manifest(reality_manifest)
    reality_run = reality_payload["workloads"][0]["runs"][1]
    assert reality_run["mode_label"] == "ait_linear_reality"
    assert reality_run["pass_or_fail"] == "fail"
    assert reality_run["comparable"] is False
    assert "Reality bootstrap AIT linear runs must use self_authored_ref." in reality_run["blocking_findings"]
    reality_report = render_static_web_hardening_task_markdown(reality_payload)
    assert "ait_linear_reality" in reality_report

    normalized_manifest = json.loads(json.dumps(base_manifest))
    normalized_manifest["bootstrap_surface"] = "normalized_execution"
    normalized_manifest["workloads"][0]["runs"].append(
        {
            "run_id": "candidate-normalized",
            "mode": "ait_linear",
            "fixture_root": str(linear_root),
            "entry_path": "index.html",
            "entry_url": "http://127.0.0.1:4673/index.html",
            "benchmark_url": "http://127.0.0.1:4673/index.html?seed=1337&bossAfter=10",
            "startup_script": "start.sh",
            "validation_script": "validate.sh",
            "startup_check_passed": True,
            "validation_check_passed": True,
            "evaluator_runtime_closeout_status": "passed",
            "topology_id": "ait-linear-single-session",
            "measured_session_count": 1,
            "provider_usage_source": "codex session jsonl",
            "equivalent_task_completion": True,
            "surface_reproducible": True,
            "medium_regression_passed": True,
            "required_behavior": {"game_over": True, "victory": True},
            "replay_check_status": "passed",
            "settings_check_status": "passed",
            "mobile_input_check_status": "passed",
            "shared_task_docs": SHARED_TASK_DOCS,
            "ait_linear_binding_path": "advisory_override",
            "plan_task_binding_mode": "advisory",
            "contract_files": CONTRACT_FILES,
            "rubric": {
                "regression_free_medium_behavior": 19,
                "hardening_systems_completeness": 18,
                "shared_contracts_and_release_docs": 18,
                "validation_and_startup_quality": 14,
                "code_structure_and_convergence_quality": 13,
                "benchmark_hygiene": 9,
            },
            "usage_kind": "measured",
            "usage": {"prompt_tokens": 820, "completion_tokens": 300},
        }
    )

    normalized_payload = evaluate_static_web_hardening_task_manifest(normalized_manifest)
    normalized_run = normalized_payload["workloads"][0]["runs"][1]
    assert normalized_run["mode_label"] == "ait_linear_normalized"
    assert normalized_run["pass_or_fail"] == "pass"
    assert normalized_run["comparable"] is True
    assert normalized_payload["aggregate"]["verdict"] == "supported"
    normalized_report = render_static_web_hardening_task_markdown(normalized_payload)
    assert "ait_linear_normalized" in normalized_report


def test_static_web_hardening_benchmark_resolves_live_fixture_roots_from_repo_cwd(
    tmp_path: Path,
    monkeypatch,
) -> None:
    repo_root = tmp_path
    manifest_root = repo_root / "docs" / "benchmarks"
    baseline_root = manifest_root / "fixtures" / "static_web_plane_shooter_medium_baseline_shared_docs"
    git_root = repo_root / ".ait" / "generated" / "benchmarks" / "live" / "fixtures" / "git-linear-run-1"
    for root in (baseline_root, git_root):
        _scaffold_hardening_fixture(root)

    manifest = {
        "benchmark_id": "unit-static-web-hardening-live-fixture-paths",
        "comparison_family": "core_single_session",
        "bootstrap_surface": "normalized_execution",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag_current"],
        "aggregate_candidate_mode": "ait_dag_current",
        "minimum_comparable_runs": 1,
        "baseline_fixture": {
            "fixture_id": "baseline-shooter",
            "fixture_root": "fixtures/static_web_plane_shooter_medium_baseline_shared_docs",
            "snapshot_id": "BASELINE-SNAPSHOT-20260508",
            "digest": compute_fixture_digest(baseline_root),
            "source_artifact": "fixtures/static_web_plane_shooter_medium_baseline_shared_docs.fixture.json",
            "shared_task_docs": SHARED_TASK_DOCS,
        },
        "workloads": [
            {
                "workload_id": "plane-shooter-hardening",
                "category": "long",
                "comparison_style": "reviewed_plan",
                "runs": [
                    {
                        "run_id": "baseline-1",
                        "mode": "git_linear",
                        "fixture_root": ".ait/generated/benchmarks/live/fixtures/git-linear-run-1",
                        "entry_path": "index.html",
                        "entry_url": "http://127.0.0.1:4473/index.html",
                        "benchmark_url": "http://127.0.0.1:4473/index.html?seed=1337&bossAfter=10",
                        "startup_script": "start.sh",
                        "validation_script": "validate.sh",
                        "startup_check_passed": True,
                        "validation_check_passed": True,
                        "evaluator_runtime_closeout_status": "passed",
                        "topology_id": "linear-single-session",
                        "measured_session_count": 1,
                        "provider_usage_source": "codex session jsonl",
                        "equivalent_task_completion": True,
                        "surface_reproducible": True,
                        "medium_regression_passed": True,
                        "required_behavior": {
                            "player_controls": True,
                            "enemy_spawns": True,
                            "game_over": True,
                            "victory": True,
                        },
                        "replay_check_status": "passed",
                        "settings_check_status": "passed",
                        "mobile_input_check_status": "passed",
                        "shared_task_docs": SHARED_TASK_DOCS,
                        "contract_files": CONTRACT_FILES,
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120},
                    },
                    {
                        "run_id": "candidate-1",
                        "mode": "ait_dag_current",
                        "fixture_root": ".ait/generated/benchmarks/live/fixtures/git-linear-run-1",
                        "entry_path": "index.html",
                        "entry_url": "http://127.0.0.1:4573/index.html",
                        "benchmark_url": "http://127.0.0.1:4573/index.html?seed=1337&bossAfter=10",
                        "startup_script": "start.sh",
                        "validation_script": "validate.sh",
                        "startup_check_passed": True,
                        "validation_check_passed": True,
                        "evaluator_runtime_closeout_status": "passed",
                        "topology_id": "full-dag-json-one-worker",
                        "measured_session_count": 1,
                        "provider_usage_source": "codex session jsonl",
                        "equivalent_task_completion": True,
                        "surface_reproducible": True,
                        "medium_regression_passed": True,
                        "required_behavior": {
                            "player_controls": True,
                            "enemy_spawns": True,
                            "game_over": True,
                            "victory": True,
                        },
                        "replay_check_status": "passed",
                        "settings_check_status": "passed",
                        "mobile_input_check_status": "passed",
                        "shared_task_docs": SHARED_TASK_DOCS,
                        "contract_files": CONTRACT_FILES,
                        "usage_kind": "measured",
                        "usage": {"prompt_tokens": 90, "completion_tokens": 18, "total_tokens": 108},
                    }
                ],
            }
        ],
    }
    manifest_root.mkdir(parents=True, exist_ok=True)
    manifest_path = manifest_root / "live.partial.json"
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

    monkeypatch.chdir(repo_root)
    payload = run_static_web_hardening_task_benchmark(manifest_path)
    run = payload["workloads"][0]["runs"][0]

    assert Path(run["fixture_root"]) == git_root.resolve()
    assert run["startup_script_exists"] is True
    assert run["validation_script_exists"] is True
    assert run["missing_shared_task_doc_paths"] == []
    assert run["missing_contract_paths"] == []


def test_benchmark_static_web_hardening_task_cli_writes_reports(tmp_path: Path) -> None:
    baseline_root = tmp_path / "baseline"
    run_root = tmp_path / "candidate"
    for root in (baseline_root, run_root):
        _scaffold_hardening_fixture(root)

    manifest = {
        "benchmark_id": "unit-static-web-hardening-cli",
        "comparison_family": "core_single_session",
        "bootstrap_surface": "reality",
        "baseline_mode": "git_linear",
        "candidate_modes": ["ait_dag_current"],
        "aggregate_candidate_mode": "ait_dag_current",
        "minimum_comparable_runs": 1,
        "baseline_fixture": {
            "fixture_root": str(baseline_root),
            "snapshot_id": "BASELINE-SNAPSHOT-20260508",
            "digest": compute_fixture_digest(baseline_root),
            "shared_task_docs": SHARED_TASK_DOCS,
        },
        "workloads": [
            {
                "workload_id": "plane-shooter-hardening",
                "category": "long",
                "runs": [
                    {
                        "run_id": "baseline-1",
                        "mode": "git_linear",
                        "fixture_root": str(run_root),
                        "entry_path": "index.html",
                        "entry_url": "http://127.0.0.1:4673/index.html",
                        "benchmark_url": "http://127.0.0.1:4673/index.html?seed=1337&bossAfter=10",
                        "startup_script": "start.sh",
                        "validation_script": "validate.sh",
                        "startup_check_passed": True,
                        "validation_check_passed": True,
                        "evaluator_runtime_closeout_status": "passed",
                        "equivalent_task_completion": True,
                        "surface_reproducible": True,
                        "medium_regression_passed": True,
                        "required_behavior": {"game_over": True, "victory": True},
                        "replay_check_status": "passed",
                        "settings_check_status": "passed",
                        "mobile_input_check_status": "passed",
                        "shared_task_docs": SHARED_TASK_DOCS,
                        "contract_files": CONTRACT_FILES,
                        "rubric": {
                            "regression_free_medium_behavior": 18,
                            "hardening_systems_completeness": 18,
                            "shared_contracts_and_release_docs": 18,
                            "validation_and_startup_quality": 14,
                            "code_structure_and_convergence_quality": 13,
                            "benchmark_hygiene": 9,
                        },
                        "usage_kind": "measured",
                        "measured_session_count": 1,
                        "provider_usage_source": "codex session jsonl",
                        "usage": {"prompt_tokens": 900, "completion_tokens": 320},
                    },
                    {
                        "run_id": "candidate-1",
                        "mode": "ait_dag_current",
                        "fixture_root": str(run_root),
                        "entry_path": "index.html",
                        "entry_url": "http://127.0.0.1:4773/index.html",
                        "benchmark_url": "http://127.0.0.1:4773/index.html?seed=1337&bossAfter=10",
                        "startup_script": "start.sh",
                        "validation_script": "validate.sh",
                        "startup_check_passed": True,
                        "validation_check_passed": True,
                        "evaluator_runtime_closeout_status": "passed",
                        "equivalent_task_completion": True,
                        "surface_reproducible": True,
                        "medium_regression_passed": True,
                        "required_behavior": {"game_over": True, "victory": True},
                        "replay_check_status": "passed",
                        "settings_check_status": "passed",
                        "mobile_input_check_status": "passed",
                        "shared_task_docs": SHARED_TASK_DOCS,
                        "contract_files": CONTRACT_FILES,
                        "rubric": {
                            "regression_free_medium_behavior": 19,
                            "hardening_systems_completeness": 19,
                            "shared_contracts_and_release_docs": 19,
                            "validation_and_startup_quality": 15,
                            "code_structure_and_convergence_quality": 14,
                            "benchmark_hygiene": 10,
                        },
                        "usage_kind": "measured",
                        "measured_session_count": 1,
                        "provider_usage_source": "codex session jsonl",
                        "usage": {"prompt_tokens": 820, "completion_tokens": 290},
                    },
                ],
            }
        ],
    }

    manifest_path = tmp_path / "manifest.json"
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
    result = runner.invoke(
        app,
        [
            "benchmark",
            "static-web-hardening-task",
            "--manifest",
            str(manifest_path),
            "--output-json",
            str(tmp_path / "out" / "report.json"),
            "--output-markdown",
            str(tmp_path / "out" / "report.md"),
            "--json",
        ],
        catch_exceptions=False,
    )

    assert result.exit_code == 0, result.stdout
    payload = json.loads(result.stdout)
    assert payload["aggregate"]["verdict"] == "supported"
    assert (tmp_path / "out" / "report.json").exists()
    assert "unit-static-web-hardening-cli" in (tmp_path / "out" / "report.md").read_text(encoding="utf-8")


def test_codex_fill_usage_cli_populates_static_web_hardening_dag_cost_breakdown(tmp_path: Path) -> None:
    with runner.isolated_filesystem(temp_dir=tmp_path):
        _write_codex_session_jsonl(Path("sprint-card.jsonl"))
        _write_codex_session_jsonl(Path("dag-json.jsonl"))
        _write_codex_session_jsonl(Path("worker.jsonl"))
        manifest = {
            "benchmark_id": "unit-static-web-hardening-dag-cost-import",
            "benchmark_kind": "static_web_hardening_task",
            "workload_kind": "task_dag_2d_plane_shooter_release_hardening",
            "baseline_mode": "git_linear",
            "candidate_modes": ["ait_dag_current"],
            "aggregate_candidate_mode": "ait_dag_current",
            "comparison_family": "core_single_session",
            "bootstrap_surface": "reality",
            "minimum_comparable_runs": 1,
            "workloads": [
                {
                    "workload_id": "plane-shooter-hardening",
                    "category": "long",
                    "runs": [
                        {
                            "run_id": "ait-dag-current-run-1",
                            "mode": "ait_dag_current",
                            "usage_kind": "measured",
                            "dag_cost_accounting_policy": "reported_separately",
                            "usage": {"prompt_tokens": None, "completion_tokens": None, "total_tokens": None},
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
                "ait-dag-current-run-1:sprint_card_setup=sprint-card.jsonl",
                "--run-role-session",
                "ait-dag-current-run-1:dag_json_authoring=dag-json.jsonl",
                "--run-role-session",
                "ait-dag-current-run-1:worker_execution=worker.jsonl",
                "--output-manifest",
                "filled.json",
                "--quality",
                "passed",
                "--json",
            ],
            catch_exceptions=False,
        )
        payload = json.loads(result.stdout)
        filled = json.loads(Path("filled.json").read_text(encoding="utf-8"))

    assert result.exit_code == 0, result.stdout
    run = filled["workloads"][0]["runs"][0]
    assert run["provider_usage_source"] == "codex session jsonl"
    assert run["usage"]["total_tokens"] == 156
    assert run["usage_provenance"]["session_count"] == 1
    assert "worker_execution" in run["usage_provenance"]["role_breakdown"]
    assert run["dag_cost_accounting_policy"] == "reported_separately"
    assert run["dag_cost_breakdown"]["sprint_card_setup"]["total_tokens"] == 156
    assert run["dag_cost_breakdown"]["dag_json_authoring"]["total_tokens"] == 156
    assert run["dag_cost_breakdown"]["worker_execution"]["total_tokens"] == 156
    assert run["dag_cost_provenance"]["session_count"] == 3
    assert payload["imported_runs"][0]["usage"]["total_tokens"] == 156


def test_static_web_hardening_surface_templates_parse_with_expected_surfaces() -> None:
    repo_root = Path(__file__).resolve().parents[1]
    reality_path = repo_root / "docs/benchmarks/static_web_hardening_task_reality_bootstrap_manifest_template.json"
    normalized_path = repo_root / "docs/benchmarks/static_web_hardening_task_normalized_execution_manifest_template.json"

    reality_payload = run_static_web_hardening_task_benchmark(reality_path)
    normalized_payload = run_static_web_hardening_task_benchmark(normalized_path)

    assert reality_payload["comparison_family"] == "core_single_session"
    assert reality_payload["bootstrap_surface"] == "reality"
    assert reality_payload["evidence_type"] == "operational"
    reality_linear = reality_payload["workloads"][0]["runs"][1]
    assert reality_linear["ait_linear_binding_path"] == "self_authored_ref"
    assert reality_linear["plan_task_binding_mode"] == "required"

    assert normalized_payload["comparison_family"] == "core_single_session"
    assert normalized_payload["bootstrap_surface"] == "normalized_execution"
    assert normalized_payload["evidence_type"] == "operational"
    normalized_linear = normalized_payload["workloads"][0]["runs"][1]
    assert normalized_linear["ait_linear_binding_path"] == "advisory_override"
    assert normalized_linear["plan_task_binding_mode"] == "advisory"
    normalized_dag = normalized_payload["workloads"][0]["runs"][2]
    assert normalized_dag["dag_cost_accounting_policy"] == "reported_separately"
