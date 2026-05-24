from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path
from typing import Any

from ait_protocol.common import (
    extract_plan_items,
    extract_plan_section,
    list_plan_section_refs,
    normalize_optional_text,
)

_MARKDOWN_HEADING_RE = re.compile(r"^(#{1,6})\s+(.*?)\s*$")
_HEADER_FIELD_RE = re.compile(r"^(Authority|Status|Scope):\s*(.+?)\s*$", re.IGNORECASE)
_REF_RE = re.compile(r"\[ref:\s*([A-Za-z0-9][A-Za-z0-9._/-]*)\]")
_LIST_BULLET_RE = re.compile(r"^\s*(?:[-*+]|\d+\.)\s+(?P<text>.+?)\s*$")

_NODE_TEMPLATE_FAMILIES: list[dict[str, str]] = [
    {
        "family_id": "schema_record",
        "label": "Schema / record work",
        "default_dependency_rule": "foundation_first",
        "default_risk_tier": "medium",
        "acceptance_focus": "stable_ids_and_serialized_shape",
    },
    {
        "family_id": "command_contract",
        "label": "Command-contract work",
        "default_dependency_rule": "after_foundation_contract",
        "default_risk_tier": "medium",
        "acceptance_focus": "deterministic_cli_or_ir_contract",
    },
    {
        "family_id": "validation_acceptance",
        "label": "Validation / acceptance work",
        "default_dependency_rule": "after_contract_and_seed",
        "default_risk_tier": "medium",
        "acceptance_focus": "measured_or_asserted_acceptance_signal",
    },
    {
        "family_id": "artifact_build_output",
        "label": "Artifact / build-output work",
        "default_dependency_rule": "after_contract",
        "default_risk_tier": "medium",
        "acceptance_focus": "reusable_compact_output",
    },
    {
        "family_id": "policy_rollback_boundary",
        "label": "Policy / rollback / boundary work",
        "default_dependency_rule": "terminal_gate",
        "default_risk_tier": "high",
        "acceptance_focus": "explicit_boundary_and_revert_story",
    },
]

_CONTRACT_FAMILY_MAP = {
    "sprint-compiler-artifact-bundle": "artifact_build_output",
    "sprint-compiler-planning-ir": "schema_record",
    "sprint-compiler-node-templates": "schema_record",
    "sprint-compiler-delta-compile": "command_contract",
    "sprint-compiler-graph-seed": "schema_record",
    "sprint-compiler-planning-rerun": "validation_acceptance",
}


def _json_digest(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()


def _text_digest(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _heading_title(raw: str) -> str:
    return re.sub(r"\s*\[(?:plan-ref|ref):.*?\]\s*", "", raw).strip()


def _header_fields(markdown: str) -> dict[str, str | None]:
    values: dict[str, str | None] = {"authority": None, "status": None, "scope": None}
    for raw_line in markdown.splitlines()[:40]:
        match = _HEADER_FIELD_RE.match(raw_line.strip())
        if match is None:
            continue
        key = str(match.group(1)).lower()
        values[key] = match.group(2).strip()
    return values


def _section_blocks(section_markdown: str, *, base_line_number: int = 1) -> list[dict[str, Any]]:
    lines = section_markdown.splitlines()
    blocks: list[dict[str, Any]] = []
    current: dict[str, Any] | None = None
    for offset, raw_line in enumerate(lines, start=0):
        line_number = base_line_number + offset
        heading_match = _MARKDOWN_HEADING_RE.match(raw_line)
        if heading_match is None:
            if current is not None:
                current["body_lines"].append(raw_line)
                current["line_end"] = line_number
            continue
        if current is not None:
            current["body"] = "\n".join(current.pop("body_lines")).strip()
            blocks.append(current)
        current = {
            "title": _heading_title(heading_match.group(2)),
            "level": len(heading_match.group(1)),
            "line_start": line_number,
            "line_end": line_number,
            "body_lines": [],
        }
    if current is not None:
        current["body"] = "\n".join(current.pop("body_lines")).strip()
        blocks.append(current)
    return blocks


def _block_by_title(blocks: list[dict[str, Any]], title: str) -> dict[str, Any] | None:
    normalized = title.strip().lower()
    for block in blocks:
        if str(block.get("title") or "").strip().lower() == normalized:
            return block
    return None


def _extract_bullets(body: str | None) -> list[str]:
    bullets: list[str] = []
    for raw_line in str(body or "").splitlines():
        match = _LIST_BULLET_RE.match(raw_line)
        if match is None:
            continue
        text = _heading_title(match.group("text"))
        if text:
            bullets.append(text)
    return bullets


def _parallel_acceptance_map(section_markdown: str) -> dict[str, list[str]]:
    acceptance: dict[str, list[str]] = {}
    capture = False
    current_ref: str | None = None
    for raw_line in section_markdown.splitlines():
        stripped = raw_line.strip()
        ref_match = _REF_RE.search(stripped)
        if ref_match is not None:
            current_ref = ref_match.group(1).strip()
            acceptance.setdefault(current_ref, [])
        if stripped.lower().startswith("acceptance:"):
            capture = True
            continue
        heading_match = _MARKDOWN_HEADING_RE.match(raw_line)
        if heading_match is not None and not stripped.lower().startswith("acceptance:"):
            capture = False
        if not capture:
            continue
        bullet_match = _LIST_BULLET_RE.match(raw_line)
        if bullet_match is None:
            continue
        bullet_text = _heading_title(bullet_match.group("text"))
        if not bullet_text:
            continue
        target_ref = current_ref
        bullet_ref = _REF_RE.search(raw_line)
        if bullet_ref is not None:
            target_ref = bullet_ref.group(1).strip()
        if target_ref:
            acceptance.setdefault(target_ref, []).append(bullet_text)
    return acceptance


def _node_family_for(node: dict[str, Any]) -> str:
    raw_key = str(node.get("node_kind") or "").strip()
    if raw_key == "land_gate":
        return "policy_rollback_boundary"
    for raw_hotspot in node.get("hotspot_keys") or []:
        text = str(raw_hotspot or "").strip()
        if text.startswith("contract:"):
            family = _CONTRACT_FAMILY_MAP.get(text.split(":", 1)[1])
            if family:
                return family
    title = str(node.get("title") or "").lower()
    if "benchmark" in title or "validate" in title or "acceptance" in title:
        return "validation_acceptance"
    if "policy" in title or "rollback" in title or "land" in title:
        return "policy_rollback_boundary"
    if "artifact" in title or "bundle" in title or "seed" in title or "build" in title:
        return "artifact_build_output"
    return "command_contract"


def _benchmark_packet_prompt() -> str:
    return (
        "Use only this planning-compiler packet. Judge whether the narrowed planning-only "
        "release/hardening workload still preserves 80%+ packet savings versus the long "
        "git_linear baseline. Keep packet savings separate from remote orchestration cost "
        "and state the claim caveat honestly."
    )


def _execution_packet_prompt() -> str:
    return (
        "Use only this compact DAG execution packet. Execute the currently ready task-graph "
        "work in one fresh worker session inside the resolved authoring workspace, normally "
        "the bound task worktree when the active focus already has one. Keep "
        "implementation, verification, and workflow updates grounded in the packet and live "
        "repository state; do not invent completed nodes, and stop at an explicit gate "
        "boundary or with `missing_context:` when required inputs are absent."
    )


def _render_benchmark_packet_text(
    compiler_input_bundle: dict[str, Any],
    planning_ir: dict[str, Any],
    node_templates: dict[str, Any],
    graph_seed: dict[str, Any],
    continuation_reuse: dict[str, Any],
) -> str:
    if not bool(compiler_input_bundle.get("available")):
        reason = normalize_optional_text(compiler_input_bundle.get("reason")) or "unknown"
        return f"Planning compiler packet unavailable.\nReason: {reason}"

    lines = [
        "Planning compiler packet",
        f"Objective: {normalize_optional_text(planning_ir.get('objective')) or 'unknown'}",
        f"Workload class: {normalize_optional_text(planning_ir.get('workload_class')) or 'unknown'}",
    ]
    artifact_path = normalize_optional_text(compiler_input_bundle.get("artifact_path"))
    artifact_selector = normalize_optional_text(compiler_input_bundle.get("artifact_selector"))
    if artifact_path:
        source_line = artifact_path
        if artifact_selector:
            source_line = f"{source_line}#{artifact_selector}"
        lines.append(f"Source: {source_line}")
    plan_revision_id = normalize_optional_text(compiler_input_bundle.get("plan_revision_id"))
    if plan_revision_id:
        lines.append(f"Plan revision: {plan_revision_id}")
    for key, label in (("authority", "Authority"), ("status", "Status"), ("scope", "Scope")):
        value = normalize_optional_text(compiler_input_bundle.get(key))
        if value:
            lines.append(f"{label}: {value}")
    stable_refs = [
        str(ref).strip()
        for ref in compiler_input_bundle.get("stable_refs") or []
        if str(ref).strip()
    ]
    if stable_refs:
        lines.append("Stable refs: " + ", ".join(stable_refs))
    guardrails = [
        str(item).strip()
        for item in (planning_ir.get("benchmark_notes") or {}).get("guardrails", [])
        if str(item).strip()
    ]
    if guardrails:
        lines.append("Guardrails:")
        lines.extend(f"- {item}" for item in guardrails[:4])
    lines.append("Work items:")
    for item in planning_ir.get("work_items") or []:
        node_id = str(item.get("node_id") or "").strip() or "?"
        title = normalize_optional_text(item.get("title")) or "untitled"
        depends_on = [str(dep).strip() for dep in item.get("depends_on") or [] if str(dep).strip()]
        family = normalize_optional_text(item.get("node_family")) or "unknown"
        plan_item_ref = normalize_optional_text(item.get("plan_item_ref")) or "-"
        dep_text = ",".join(depends_on) if depends_on else "-"
        lines.append(f"- {node_id}: {title} | deps:{dep_text} | family:{family} | ref:{plan_item_ref}")
    completion_gates = planning_ir.get("completion_gates") or []
    if completion_gates:
        lines.append("Completion gates:")
        for entry in completion_gates[:8]:
            gate = normalize_optional_text(entry.get("gate")) or normalize_optional_text(entry.get("title")) or "gate"
            node_id = normalize_optional_text(entry.get("node_id")) or "?"
            lines.append(f"- {node_id}: {gate}")
    family_ids = [
        str(row.get("family_id") or "").strip()
        for row in node_templates.get("template_families") or []
        if str(row.get("family_id") or "").strip()
    ]
    if family_ids:
        lines.append("Template families: " + ", ".join(family_ids))
    graph_id = normalize_optional_text(graph_seed.get("graph_id")) or "unknown"
    node_count = int(graph_seed.get("node_count") or 0)
    edge_count = int(graph_seed.get("edge_count") or 0)
    graph_digest = normalize_optional_text(graph_seed.get("seed_digest")) or "-"
    lines.append(f"Graph seed: {graph_id} | nodes:{node_count} | edges:{edge_count} | digest:{graph_digest}")
    reuse_key = normalize_optional_text(continuation_reuse.get("reuse_key"))
    if reuse_key:
        lines.append(f"Reuse key: {reuse_key}")
    bundle_digest = normalize_optional_text(compiler_input_bundle.get("bundle_digest")) or "-"
    ir_digest = normalize_optional_text(planning_ir.get("ir_digest")) or "-"
    lines.append(f"Digests: bundle={bundle_digest} ir={ir_digest}")
    return "\n".join(lines)


def _render_execution_packet_text(
    compiler_input_bundle: dict[str, Any],
    planning_ir: dict[str, Any],
    node_templates: dict[str, Any],
    graph_seed: dict[str, Any],
    continuation_reuse: dict[str, Any],
) -> str:
    if not bool(compiler_input_bundle.get("available")):
        reason = normalize_optional_text(compiler_input_bundle.get("reason")) or "unknown"
        return f"Compact DAG execution packet unavailable.\nReason: {reason}"

    lines = [
        "Compact DAG execution packet",
        f"Objective: {normalize_optional_text(planning_ir.get('objective')) or 'unknown'}",
        f"Workload class: {normalize_optional_text(planning_ir.get('workload_class')) or 'unknown'}",
    ]
    artifact_path = normalize_optional_text(compiler_input_bundle.get("artifact_path"))
    artifact_selector = normalize_optional_text(compiler_input_bundle.get("artifact_selector"))
    if artifact_path:
        source_line = artifact_path
        if artifact_selector:
            source_line = f"{source_line}#{artifact_selector}"
        lines.append(f"Source: {source_line}")
    plan_revision_id = normalize_optional_text(compiler_input_bundle.get("plan_revision_id"))
    if plan_revision_id:
        lines.append(f"Plan revision: {plan_revision_id}")
    for key, label in (("authority", "Authority"), ("status", "Status"), ("scope", "Scope")):
        value = normalize_optional_text(compiler_input_bundle.get(key))
        if value:
            lines.append(f"{label}: {value}")
    stable_refs = [
        str(ref).strip()
        for ref in compiler_input_bundle.get("stable_refs") or []
        if str(ref).strip()
    ]
    if stable_refs:
        lines.append("Stable refs: " + ", ".join(stable_refs))
    dispatch_intent = normalize_optional_text(planning_ir.get("dispatch_intent"))
    if dispatch_intent:
        lines.extend(["Dispatch intent:", dispatch_intent])
    lines.extend(
        [
            "Execution guardrails:",
            "- Use this packet as the planning source of truth for the current graph scope.",
            "- Use repository files only as needed to implement or verify the currently ready work.",
            "- Keep review, attestation, policy, and land explicit; do not report those gates passed unless they were actually run.",
            "- If the packet or workspace is insufficient, reply with `missing_context:` instead of silently broadening scope.",
        ]
    )
    lines.append("Work items:")
    for item in planning_ir.get("work_items") or []:
        node_id = str(item.get("node_id") or "").strip() or "?"
        title = normalize_optional_text(item.get("title")) or "untitled"
        depends_on = [str(dep).strip() for dep in item.get("depends_on") or [] if str(dep).strip()]
        plan_item_ref = normalize_optional_text(item.get("plan_item_ref")) or "-"
        workflow_boundary = normalize_optional_text(item.get("workflow_boundary")) or "task"
        hotspots = [str(dep).strip() for dep in item.get("hotspot_keys") or [] if str(dep).strip()]
        dep_text = ",".join(depends_on) if depends_on else "-"
        hotspot_text = ",".join(hotspots) if hotspots else "-"
        lines.append(
            f"- {node_id}: {title} | deps:{dep_text} | boundary:{workflow_boundary} | ref:{plan_item_ref} | hotspots:{hotspot_text}"
        )
    completion_gates = planning_ir.get("completion_gates") or []
    if completion_gates:
        lines.append("Completion gates:")
        for entry in completion_gates[:8]:
            gate = normalize_optional_text(entry.get("gate")) or normalize_optional_text(entry.get("title")) or "gate"
            node_id = normalize_optional_text(entry.get("node_id")) or "?"
            lines.append(f"- {node_id}: {gate}")
    family_ids = [
        str(row.get("family_id") or "").strip()
        for row in node_templates.get("template_families") or []
        if str(row.get("family_id") or "").strip()
    ]
    if family_ids:
        lines.append("Template families: " + ", ".join(family_ids))
    graph_id = normalize_optional_text(graph_seed.get("graph_id")) or "unknown"
    node_count = int(graph_seed.get("node_count") or 0)
    edge_count = int(graph_seed.get("edge_count") or 0)
    graph_digest = normalize_optional_text(graph_seed.get("seed_digest")) or "-"
    lines.append(f"Graph seed: {graph_id} | nodes:{node_count} | edges:{edge_count} | digest:{graph_digest}")
    reuse_key = normalize_optional_text(continuation_reuse.get("reuse_key"))
    if reuse_key:
        lines.append(f"Reuse key: {reuse_key}")
    bundle_digest = normalize_optional_text(compiler_input_bundle.get("bundle_digest")) or "-"
    ir_digest = normalize_optional_text(planning_ir.get("ir_digest")) or "-"
    lines.append(f"Digests: bundle={bundle_digest} ir={ir_digest}")
    return "\n".join(lines)


def _missing_surface(graph: dict[str, Any], *, reason: str, artifact_path: str | None, artifact_selector: str | None) -> dict[str, Any]:
    nodes = graph.get("nodes") if isinstance(graph.get("nodes"), list) else []
    edges = graph.get("edges") if isinstance(graph.get("edges"), list) else []
    graph_seed = {
        "graph_id": normalize_optional_text(graph.get("graph_id")) or "unknown",
        "node_count": len(nodes),
        "edge_count": len(edges),
        "nodes": nodes,
        "edges": edges,
    }
    graph_seed["seed_digest"] = _json_digest(graph_seed)
    compiler_input_bundle = {
        "available": False,
        "reason": reason,
        "artifact_path": artifact_path,
        "artifact_selector": artifact_selector,
    }
    planning_ir = {
        "available": False,
        "objective": "unavailable",
        "workload_class": "unknown",
        "work_items": [],
        "dependencies": [],
        "completion_gates": [],
        "ir_digest": _json_digest({"available": False, "reason": reason}),
    }
    node_templates = {
        "template_families": list(_NODE_TEMPLATE_FAMILIES),
        "node_assignments": [],
        "template_digest": _json_digest(_NODE_TEMPLATE_FAMILIES),
    }
    continuation_reuse = {
        "available": False,
        "delta_supported": False,
        "reuse_key": "",
    }
    benchmark_context_text = _render_benchmark_packet_text(
        compiler_input_bundle,
        planning_ir,
        node_templates,
        graph_seed,
        continuation_reuse,
    )
    benchmark_prompt_text = _benchmark_packet_prompt()
    benchmark_packet = {
        "mode": "planning_compiler",
        "context_text": benchmark_context_text,
        "prompt_text": benchmark_prompt_text,
        "context_char_count": len(benchmark_context_text),
        "prompt_char_count": len(benchmark_prompt_text),
        "context_digest": _text_digest(benchmark_context_text),
        "prompt_digest": _text_digest(benchmark_prompt_text),
        "comparison_boundary": "packet_only_excludes_remote_orchestration",
        "workload_class": planning_ir.get("workload_class"),
    }
    execution_context_text = _render_execution_packet_text(
        compiler_input_bundle,
        planning_ir,
        node_templates,
        graph_seed,
        continuation_reuse,
    )
    execution_prompt_text = _execution_packet_prompt()
    execution_packet = {
        "mode": "task_dag_execution",
        "context_text": execution_context_text,
        "prompt_text": execution_prompt_text,
        "context_char_count": len(execution_context_text),
        "prompt_char_count": len(execution_prompt_text),
        "context_digest": _text_digest(execution_context_text),
        "prompt_digest": _text_digest(execution_prompt_text),
        "workspace_mode": "authoring_workspace",
        "workload_class": planning_ir.get("workload_class"),
    }
    return {
        "schema_version": 1,
        "compiler_input_bundle": compiler_input_bundle,
        "planning_ir": planning_ir,
        "node_templates": node_templates,
        "graph_seed": graph_seed,
        "continuation_reuse": continuation_reuse,
        "benchmark_packet": benchmark_packet,
        "execution_packet": execution_packet,
    }


def build_task_dag_planning_compiler_surface(
    repo_root: Path,
    graph: dict[str, Any],
    *,
    graph_path: Path | None = None,
) -> dict[str, Any]:
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    artifact_path = normalize_optional_text(source_plan.get("artifact_path") or graph.get("artifact_path"))
    artifact_selector = normalize_optional_text(
        source_plan.get("artifact_selector")
        or source_plan.get("plan_ref")
        or graph.get("artifact_selector")
        or graph.get("plan_ref")
    )
    plan_revision_id = normalize_optional_text(source_plan.get("plan_revision_id") or graph.get("plan_revision_id"))
    if artifact_path is None:
        return _missing_surface(graph, reason="missing_artifact_path", artifact_path=None, artifact_selector=artifact_selector)

    repo_root = Path(repo_root)
    resolved_artifact = Path(artifact_path)
    if not resolved_artifact.is_absolute():
        resolved_artifact = repo_root / resolved_artifact
    if not resolved_artifact.is_file():
        return _missing_surface(graph, reason="artifact_not_found", artifact_path=artifact_path, artifact_selector=artifact_selector)

    markdown = resolved_artifact.read_text(encoding="utf-8")
    header_fields = _header_fields(markdown)
    section = extract_plan_section(markdown, artifact_selector)
    if section is None:
        section_markdown = markdown
        section_line_start = 1
        heading_title = resolved_artifact.stem
    else:
        section_markdown = str(section.get("section_markdown") or "")
        section_line_start = int(section.get("line_number") or 1)
        heading_title = normalize_optional_text(section.get("heading_title")) or resolved_artifact.stem

    section_digest = _text_digest(section_markdown)
    artifact_blob_id = _text_digest(markdown)
    items = extract_plan_items(section_markdown)
    item_lookup = {item["plan_item_ref"]: item for item in items if item.get("plan_item_ref")}
    acceptance_lookup = _parallel_acceptance_map(section_markdown)
    blocks = _section_blocks(section_markdown, base_line_number=section_line_start)
    known_refs = list_plan_section_refs(markdown)
    stable_refs = sorted(
        {
            *(str(item.get("plan_item_ref")) for item in items if item.get("plan_item_ref")),
            *(str(ref.get("plan_ref")) for ref in known_refs if ref.get("plan_ref")),
        }
    )

    nodes = graph.get("nodes") if isinstance(graph.get("nodes"), list) else []
    edges = graph.get("edges") if isinstance(graph.get("edges"), list) else []
    dependencies: list[dict[str, str]] = []
    for edge in edges:
        if not isinstance(edge, dict):
            continue
        from_node = normalize_optional_text(edge.get("from") or edge.get("source"))
        to_node = normalize_optional_text(edge.get("to") or edge.get("target"))
        if from_node and to_node:
            dependencies.append({"from": from_node, "to": to_node, "edge_kind": normalize_optional_text(edge.get("edge_kind")) or "depends_on"})

    work_items: list[dict[str, Any]] = []
    completion_gates: list[dict[str, Any]] = []
    node_assignments: list[dict[str, Any]] = []
    for node in nodes:
        if not isinstance(node, dict):
            continue
        node_id = normalize_optional_text(node.get("node_id") or node.get("id")) or "unknown"
        plan_item_ref = normalize_optional_text(node.get("plan_item_ref"))
        item = item_lookup.get(plan_item_ref or "")
        title = normalize_optional_text(node.get("title")) or normalize_optional_text(item.get("text") if item else None) or node_id
        depends_on = [str(dep).strip() for dep in node.get("depends_on") or [] if str(dep).strip()]
        for dependency in dependencies:
            if dependency["to"] == node_id and dependency["from"] not in depends_on:
                depends_on.append(dependency["from"])
        node_family = _node_family_for(node)
        intent = (
            normalize_optional_text(node.get("intent"))
            or normalize_optional_text((node.get("task_template") or {}).get("intent") if isinstance(node.get("task_template"), dict) else None)
            or title
        )
        acceptance = list(acceptance_lookup.get(plan_item_ref or "", []))
        if not acceptance and node.get("completion_rule"):
            acceptance.append(str(node.get("completion_rule")))
        if not acceptance:
            acceptance.append(f"{title} produces a reviewable output or explicit blocker.")
        work_item = {
            "node_id": node_id,
            "title": title,
            "plan_item_ref": plan_item_ref,
            "node_family": node_family,
            "depends_on": depends_on,
            "intent": intent,
            "acceptance": acceptance,
            "workflow_boundary": normalize_optional_text(node.get("workflow_boundary"))
            or ("policy_rollback_boundary" if str(node.get("node_kind") or "").strip() == "land_gate" else "task"),
            "hotspot_keys": [str(item).strip() for item in node.get("hotspot_keys") or [] if str(item).strip()],
        }
        work_items.append(work_item)
        node_assignments.append(
            {
                "node_id": node_id,
                "family_id": node_family,
                "risk_tier": "high" if node_family == "policy_rollback_boundary" else "medium",
                "acceptance_focus": next(
                    (row["acceptance_focus"] for row in _NODE_TEMPLATE_FAMILIES if row["family_id"] == node_family),
                    "reviewable_output",
                ),
            }
        )
        for gate in acceptance:
            completion_gates.append(
                {
                    "node_id": node_id,
                    "title": title,
                    "plan_item_ref": plan_item_ref,
                    "gate": gate,
                }
            )

    execution_headings = [
        block["title"]
        for block in blocks
        if str(block.get("title") or "").strip()
    ][:8]
    guardrails = [
        "Provider-measured token usage only; do not restore estimated token claims.",
        "Keep packet-only savings separate from coordinator or remote orchestration cost.",
        "Do not bypass review, attestation, policy, or land gates.",
        "Report not-run checks honestly.",
    ]
    for title in ("Safety and interpretation rules", "Benchmark contract", "Current measured result"):
        block = _block_by_title(blocks, title)
        guardrails.extend(_extract_bullets(block.get("body") if block else None)[:4])
    dispatch_intent_block = _block_by_title(blocks, "Dispatch intent")

    planning_ir = {
        "available": True,
        "objective": heading_title,
        "workload_class": "long",
        "artifact_path": artifact_path,
        "artifact_selector": artifact_selector,
        "plan_revision_id": plan_revision_id,
        "section_line_start": section_line_start,
        "execution_headings": execution_headings,
        "dispatch_intent": normalize_optional_text(dispatch_intent_block.get("body") if dispatch_intent_block else None),
        "work_items": work_items,
        "dependencies": dependencies,
        "completion_gates": completion_gates,
        "benchmark_notes": {"guardrails": guardrails[:12]},
    }
    planning_ir["ir_digest"] = _json_digest(planning_ir)

    node_templates = {
        "template_families": list(_NODE_TEMPLATE_FAMILIES),
        "node_assignments": node_assignments,
    }
    node_templates["template_digest"] = _json_digest(node_templates)

    graph_seed = {
        "graph_id": normalize_optional_text(graph.get("graph_id")) or "unknown",
        "graph_path": str(graph_path) if graph_path is not None else None,
        "source_plan": dict(source_plan),
        "node_count": len(nodes),
        "edge_count": len(dependencies),
        "nodes": [
            {
                "node_id": item["node_id"],
                "title": item["title"],
                "plan_item_ref": item["plan_item_ref"],
                "node_family": item["node_family"],
                "depends_on": item["depends_on"],
                "workflow_boundary": item["workflow_boundary"],
            }
            for item in work_items
        ],
        "edges": dependencies,
    }
    graph_seed["seed_digest"] = _json_digest(graph_seed)

    compiler_input_bundle = {
        "available": True,
        "artifact_path": artifact_path,
        "artifact_selector": artifact_selector,
        "plan_revision_id": plan_revision_id,
        "authority": header_fields.get("authority"),
        "status": header_fields.get("status"),
        "scope": header_fields.get("scope"),
        "artifact_heading": heading_title,
        "stable_refs": stable_refs,
        "artifact_blob_id": artifact_blob_id,
        "section_digest": section_digest,
    }
    compiler_input_bundle["bundle_digest"] = _json_digest(compiler_input_bundle)

    continuation_reuse = {
        "available": True,
        "delta_supported": True,
        "reuse_key": ":".join(
            part
            for part in (
                artifact_path or "",
                artifact_selector or "",
                plan_revision_id or "",
                section_digest or planning_ir["ir_digest"],
            )
            if part
        ),
        "reusable_components": [
            "compiler_input_bundle",
            "planning_ir",
            "node_templates",
            "graph_seed",
        ],
        "invalidation_inputs": {
            "artifact_blob_id": artifact_blob_id,
            "plan_revision_id": plan_revision_id,
            "stable_ref_count": len(stable_refs),
            "node_count": graph_seed["node_count"],
            "edge_count": graph_seed["edge_count"],
        },
        "bundle_digest": compiler_input_bundle["bundle_digest"],
        "planning_ir_digest": planning_ir["ir_digest"],
        "template_digest": node_templates["template_digest"],
        "graph_seed_digest": graph_seed["seed_digest"],
        "continuation_packet": {
            "graph_id": graph_seed["graph_id"],
            "artifact_path": artifact_path,
            "artifact_selector": artifact_selector,
            "plan_revision_id": plan_revision_id,
            "bundle_digest": compiler_input_bundle["bundle_digest"],
            "planning_ir_digest": planning_ir["ir_digest"],
            "graph_seed_digest": graph_seed["seed_digest"],
            "template_family_ids": [row["family_id"] for row in _NODE_TEMPLATE_FAMILIES],
        },
    }

    benchmark_packet_text = _render_benchmark_packet_text(
        compiler_input_bundle,
        planning_ir,
        node_templates,
        graph_seed,
        continuation_reuse,
    )
    benchmark_prompt_text = _benchmark_packet_prompt()
    benchmark_packet = {
        "mode": "planning_compiler",
        "context_text": benchmark_packet_text,
        "prompt_text": benchmark_prompt_text,
        "context_char_count": len(benchmark_packet_text),
        "prompt_char_count": len(benchmark_prompt_text),
        "context_digest": _text_digest(benchmark_packet_text),
        "prompt_digest": _text_digest(benchmark_prompt_text),
        "comparison_boundary": "packet_only_excludes_remote_orchestration",
        "workload_class": planning_ir.get("workload_class"),
    }
    execution_packet_text = _render_execution_packet_text(
        compiler_input_bundle,
        planning_ir,
        node_templates,
        graph_seed,
        continuation_reuse,
    )
    execution_prompt_text = _execution_packet_prompt()
    execution_packet = {
        "mode": "task_dag_execution",
        "context_text": execution_packet_text,
        "prompt_text": execution_prompt_text,
        "context_char_count": len(execution_packet_text),
        "prompt_char_count": len(execution_prompt_text),
        "context_digest": _text_digest(execution_packet_text),
        "prompt_digest": _text_digest(execution_prompt_text),
        "workspace_mode": "authoring_workspace",
        "workload_class": planning_ir.get("workload_class"),
    }

    return {
        "schema_version": 1,
        "compiler_input_bundle": compiler_input_bundle,
        "planning_ir": planning_ir,
        "node_templates": node_templates,
        "graph_seed": graph_seed,
        "continuation_reuse": continuation_reuse,
        "benchmark_packet": benchmark_packet,
        "execution_packet": execution_packet,
    }
