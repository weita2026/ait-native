from __future__ import annotations

import json
from pathlib import Path


def test_public_demo_data_contract_paths_and_boundaries() -> None:
    payload = json.loads(Path("docs/public_demo_data_contract.json").read_text(encoding="utf-8"))

    assert payload["schema_version"] == 1
    assert payload["guide_path"] == "docs/PUBLIC_DEMO_DATA.md"
    assert payload["demo_surface"]["mode"] == "repo_contained_public_demo_pack"
    assert payload["demo_surface"]["requires_private_ait_state"] is False
    assert payload["demo_surface"]["separate_example_repo_promised"] is False

    assert payload["read_order"][:3] == [
        "README.md",
        "docs/LOCAL_QUICKSTART.md",
        "docs/PUBLIC_DEMO_DATA.md",
    ]
    assert payload["related_guides"] == [
        "README.md",
        "docs/LOCAL_QUICKSTART.md",
        "docs/SELF_HOSTED_TEAM_DEPLOYMENT.md",
        "docs/COMPATIBILITY_MATRIX.md",
        "docs/PACKAGE_TARGETS.md",
        "docs/benchmarks/README.md",
    ]

    for bundle in payload["demo_bundles"]:
        for path in bundle["artifact_paths"]:
            assert Path(path).exists(), path


def test_public_demo_data_contract_matches_committed_m3_metrics() -> None:
    payload = json.loads(Path("docs/public_demo_data_contract.json").read_text(encoding="utf-8"))
    compact_status = json.loads(
        Path("docs/benchmarks/m3_one_hour_sprint_20_node_measured_20260423_status.json").read_text(
            encoding="utf-8"
        )
    )
    compact_report = json.loads(
        Path("docs/benchmarks/m3_one_hour_sprint_20_node_measured_20260423_report.json").read_text(
            encoding="utf-8"
        )
    )
    physical_report = json.loads(
        Path("docs/benchmarks/m3_one_hour_sprint_20_node_physical_fanout_20260423_report.json").read_text(
            encoding="utf-8"
        )
    )

    bundles = {bundle["bundle_id"]: bundle for bundle in payload["demo_bundles"]}
    compact = bundles["provider_measured_compact_dag"]
    physical = bundles["physical_fanout_followup"]

    compact_runs = {run["mode"]: run for run in compact_status["runs"]}

    assert compact["claim_ready"] == compact_report["aggregate"]["claim_ready"] is False
    assert compact["headline_metrics"]["measured_ready_runs"] == compact_status["summary"]["measured_ready_count"]
    assert compact["headline_metrics"]["git_linear_total_tokens"] == compact_runs["git_linear"]["total_tokens"]
    assert compact["headline_metrics"]["ait_linear_total_tokens"] == compact_runs["ait_linear"]["total_tokens"]
    assert compact["headline_metrics"]["ait_dag_total_tokens"] == compact_runs["ait_dag"]["total_tokens"]
    assert compact["headline_metrics"]["ait_dag_savings_percent_vs_git_linear"] == compact_report["aggregate"][
        "long_ait_dag_median_saving_percent"
    ]

    assert physical["claim_ready"] == physical_report["claim_ready"] is False
    assert physical["headline_metrics"]["passed_nodes"] == physical_report["summary"]["passed_node_count"]
    assert physical["headline_metrics"]["total_nodes"] == physical_report["summary"]["node_count"]
    assert physical["headline_metrics"]["physical_fanout_total_tokens"] == physical_report["summary"][
        "total_tokens"
    ]
    assert physical["headline_metrics"]["physical_vs_single_packet_multiplier"] == physical_report[
        "comparisons"
    ]["vs_ait_dag_single_packet"]["cost_multiplier"]
