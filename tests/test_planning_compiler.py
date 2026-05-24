from __future__ import annotations

import json
from pathlib import Path

from ait.planning_compiler import build_task_dag_planning_compiler_surface


def _graph(artifact_path: str = "docs/sprints/demo.md") -> dict:
    return {
        "graph_id": "demo/task-dag",
        "source_plan": {
            "artifact_path": artifact_path,
            "artifact_selector": "demo/root",
            "plan_ref": "demo/root",
            "plan_id": "PL-demo",
            "plan_revision_id": "PR-demo",
        },
        "nodes": [
            {
                "node_id": "A",
                "node_kind": "task",
                "title": "Artifact bundle",
                "plan_item_ref": "demo/a",
                "depends_on": [],
                "hotspot_keys": ["contract:sprint-compiler-artifact-bundle"],
                "task_template": {"intent": "Resolve the bundle first."},
            },
            {
                "node_id": "B",
                "node_kind": "task",
                "title": "Planning IR",
                "plan_item_ref": "demo/b",
                "depends_on": ["A"],
                "hotspot_keys": ["contract:sprint-compiler-planning-ir"],
                "task_template": {"intent": "Compile the IR next."},
            },
            {
                "node_id": "L",
                "node_kind": "land_gate",
                "title": "Land gate",
                "plan_item_ref": "demo/land",
                "depends_on": ["B"],
                "completion_rule": "selected patchset lands cleanly",
            },
        ],
        "edges": [
            {"from": "A", "to": "B", "edge_kind": "depends_on"},
            {"from": "B", "to": "L", "edge_kind": "depends_on"},
        ],
    }


def test_build_task_dag_planning_compiler_surface_reads_markdown_artifact(tmp_path: Path) -> None:
    artifact = tmp_path / "docs" / "sprints" / "demo.md"
    artifact.parent.mkdir(parents=True, exist_ok=True)
    artifact.write_text(
        "\n".join(
            [
                "# Demo",
                "",
                "Authority: demo authority",
                "Status: demo status",
                "Scope: demo scope",
                "",
                "## Demo Root [plan-ref: demo/root]",
                "",
                "- [ ] Build artifact bundle [ref: demo/a]",
                "- [ ] Compile planning IR [ref: demo/b]",
                "- [ ] Land safely [ref: demo/land]",
                "",
                "Acceptance:",
                "",
                "- Bundle has stable refs. [ref: demo/a]",
                "- IR has deterministic shape. [ref: demo/b]",
                "",
                "### Safety and interpretation rules",
                "",
                "- Keep claims measured-only.",
            ]
        ),
        encoding="utf-8",
    )
    graph_path = tmp_path / "demo.task_graph.json"
    graph_path.write_text(json.dumps(_graph(), indent=2), encoding="utf-8")

    payload = build_task_dag_planning_compiler_surface(tmp_path, _graph(), graph_path=graph_path)

    assert payload["compiler_input_bundle"]["available"] is True
    assert "demo/a" in payload["compiler_input_bundle"]["stable_refs"]
    assert "demo/b" in payload["compiler_input_bundle"]["stable_refs"]
    assert payload["compiler_input_bundle"]["authority"] == "demo authority"
    assert len(payload["planning_ir"]["work_items"]) == 3
    assert payload["planning_ir"]["work_items"][0]["acceptance"] == ["Bundle has stable refs."]
    assert payload["planning_ir"]["work_items"][1]["depends_on"] == ["A"]
    assert payload["node_templates"]["node_assignments"][0]["family_id"] == "artifact_build_output"
    assert payload["graph_seed"]["edge_count"] == 2
    assert payload["continuation_reuse"]["delta_supported"] is True
    assert payload["benchmark_packet"]["mode"] == "planning_compiler"
    assert payload["execution_packet"]["mode"] == "task_dag_execution"
    assert "resolved authoring workspace" in payload["execution_packet"]["prompt_text"]
    assert "estimated_usage_kind" not in payload["benchmark_packet"]


def test_build_task_dag_planning_compiler_surface_marks_missing_artifact_unavailable(tmp_path: Path) -> None:
    payload = build_task_dag_planning_compiler_surface(tmp_path, _graph("docs/sprints/missing.md"))

    assert payload["compiler_input_bundle"]["available"] is False
    assert payload["compiler_input_bundle"]["reason"] == "artifact_not_found"
    assert payload["planning_ir"]
    assert payload["graph_seed"]["node_count"] == 3
    assert "Planning compiler packet unavailable." in payload["benchmark_packet"]["context_text"]
    assert "Compact DAG execution packet unavailable." in payload["execution_packet"]["context_text"]
