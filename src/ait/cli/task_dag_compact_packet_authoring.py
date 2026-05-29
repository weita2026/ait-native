from __future__ import annotations

import json
import re
import shutil
import sys
import time
from pathlib import Path
from typing import Any

from ait_protocol.common import normalize_optional_text

from ..planning_compiler import (
    build_task_dag_planning_compiler_surface as _fallback_build_task_dag_planning_compiler_surface,
)
from ..store import RepoContext, current_line as _fallback_current_line, load_config as _fallback_load_config
from .task_dag_runtime_helpers import _task_dag_relative_path as _fallback_task_dag_relative_path
from .task_dag_topology_helpers import (
    _task_dag_converged_output_node_ids as _fallback_task_dag_converged_output_node_ids,
)


DEFAULT_TASK_DAG_FINAL_GATE_BUNDLE = ("review", "attestation", "policy", "land")


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is None:
        return fallback
    return getattr(app_module, name, fallback)


def build_task_dag_planning_compiler_surface(root: Path, graph: dict[str, Any], *, graph_path: Path | None = None) -> dict[str, Any]:
    builder = _app_override("build_task_dag_planning_compiler_surface", _fallback_build_task_dag_planning_compiler_surface)
    return builder(root, graph, graph_path=graph_path)


def current_line(ctx: RepoContext) -> str:
    return _app_override("current_line", _fallback_current_line)(ctx)


def load_config(ctx: RepoContext) -> dict[str, Any]:
    return _app_override("load_config", _fallback_load_config)(ctx)


def _task_dag_relative_path(ctx: RepoContext, path: Path) -> str:
    return _app_override("_task_dag_relative_path", _fallback_task_dag_relative_path)(ctx, path)


def _task_dag_converged_output_node_ids(graph: dict[str, Any]) -> list[str]:
    return _app_override("_task_dag_converged_output_node_ids", _fallback_task_dag_converged_output_node_ids)(graph)


def _task_dag_compact_packet_surface_payload(
    ctx: RepoContext,
    *,
    graph_path: Path,
    final_remote_disposition_default: bool = False,
    change_focus_policy: dict[str, Any] | None = None,
) -> dict[str, Any]:
    graph_artifact_path = _task_dag_relative_path(ctx, graph_path)
    final_land_disposition = "remote" if final_remote_disposition_default else "local"
    focus_clause = (
        " Keep exactly one reviewable focus change active at a time, cut a patchset when that focus becomes "
        "reviewable, and only then advance to the next reviewable change."
    )
    if final_remote_disposition_default:
        copyable_turn_text = (
            "Please execute this DAG using the supplied compact packet, keep execution-only work local-first, and carry "
            "the converged reviewable output through the final remote review/attestation/policy/land gates."
            + focus_clause
        )
    else:
        copyable_turn_text = (
            "Please execute this DAG using the supplied compact packet, keep execution-only work local-first, and carry "
            "the converged reviewable output through one explicit local land on the target line before you stop. After "
            "that local land, the final converged output may later remote-promote through `ait workflow land --all-completed-local --remote <name>`."
            + focus_clause
        )
    return {
        "surface_id": "worker_only_compact_ait_dag_packet",
        "label": "worker-only compact `ait_dag` packet",
        "benchmark_aligned": True,
        "packet_mode": "implementation_default",
        "graph_artifact_path": graph_artifact_path,
        "packet_generation_required": True,
        "fresh_worker_session": True,
        "worker_session_count": 1,
        "physical_fanout": False,
        "coordinator_plus_worker_end_to_end": False,
        "repo_replay_policy": "packet_scoped_only",
        "copyable_turn_text": copyable_turn_text,
        "compare_turn_text": None,
        "final_land_disposition": final_land_disposition,
        "final_remote_disposition_default": final_remote_disposition_default,
        "per_change_focus_required": True,
        "per_change_patchset_required": True,
        "shared_reviewable_diff_allowed": False,
        "change_focus_policy": change_focus_policy or {},
    }


TASK_DAG_COMPACT_PACKET_FORBIDDEN_COMMAND_PATTERNS = (
    "grep -R",
    "rg ",
    "fd ",
    "find ..",
    "find ../",
    "find /",
    "cd ..",
    "ls ..",
    "cat ..",
    "sed -n ../",
    "git status",
    "git diff",
    "git log",
)
DEFAULT_TASK_DAG_COMPACT_PACKET_MAX_COMMAND_COUNT = 2
DEFAULT_TASK_DAG_IMPLEMENTATION_PACKET_MAX_COMMAND_COUNT = 40
DEFAULT_TASK_DAG_IMPLEMENTATION_AUTO_LAND_PACKET_MAX_COMMAND_COUNT = 120
DEFAULT_TASK_DAG_COMPACT_PACKET_REPLY_POLL_TIMEOUT_SECONDS = 90
DEFAULT_TASK_DAG_IMPLEMENTATION_PACKET_REPLY_POLL_TIMEOUT_SECONDS = 240
DEFAULT_TASK_DAG_IMPLEMENTATION_AUTO_LAND_REPLY_POLL_TIMEOUT_SECONDS = 900
DEFAULT_TASK_DAG_COMPACT_PACKET_REPLY_POLL_INTERVAL_SECONDS = 2.0


def _task_dag_compact_packet_slug(value: str) -> str:
    slug = re.sub(r"[^a-z0-9]+", "-", str(value or "").strip().lower()).strip("-")
    return slug or "task-dag-packet"


def _task_dag_compact_packet_bridge_relative_path(output_dir_relative: Path, relative_repo_path: str | Path) -> str:
    return str((output_dir_relative / "authoring_workspace_context" / Path(relative_repo_path)).as_posix())


def _task_dag_compact_packet_bridge_source(
    ctx: RepoContext,
    *,
    repo_relative_path: str | None,
) -> tuple[Path, str] | None:
    relative_value = normalize_optional_text(repo_relative_path)
    if relative_value is None:
        return None
    workspace_root = ctx.root.resolve()
    repo_root = ctx.repo_root.resolve()
    candidate = Path(relative_value)
    if candidate.is_absolute():
        resolved_candidate = candidate.resolve()
        for base in (workspace_root, repo_root):
            try:
                relative_path = resolved_candidate.relative_to(base)
            except ValueError:
                continue
            return resolved_candidate, relative_path.as_posix()
        return None
    for base in (workspace_root, repo_root):
        source_path = (base / candidate).resolve()
        if source_path.is_file():
            return source_path, candidate.as_posix()
    return None


def _task_dag_compact_packet_bridge_files(
    ctx: RepoContext,
    *,
    output_dir_relative: Path,
    repo_relative_paths: list[str],
    seen_hints: set[str] | None = None,
) -> tuple[list[tuple[Path, str]], list[str]]:
    seen = seen_hints if seen_hints is not None else set()
    bridge_specs: list[tuple[Path, str]] = []
    bridge_hints: list[str] = []
    for repo_relative_path in repo_relative_paths:
        bridge = _task_dag_compact_packet_bridge_source(
            ctx,
            repo_relative_path=repo_relative_path,
        )
        if bridge is None:
            continue
        source_path, relative_repo_path = bridge
        hint = _task_dag_compact_packet_bridge_relative_path(output_dir_relative, relative_repo_path)
        if hint in seen:
            continue
        seen.add(hint)
        bridge_specs.append((source_path, hint))
        bridge_hints.append(hint)
    return bridge_specs, bridge_hints


def _task_dag_compact_packet_focus_is_reviewable_output(current_focus: dict[str, Any] | None) -> bool:
    if not isinstance(current_focus, dict):
        return False
    return normalize_optional_text(current_focus.get("workflow_boundary")) == "reviewable_output"


def _task_dag_compact_packet_current_focus_excerpt_text(
    *,
    plan_id: str,
    graph: dict[str, Any],
    current_focus: dict[str, Any],
    source_artifact_path: str | None,
) -> str:
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    source_artifact = normalize_optional_text(source_artifact_path) or normalize_optional_text(source_plan.get("artifact_path"))
    source_plan_ref = normalize_optional_text(source_plan.get("plan_ref"))
    title = normalize_optional_text(current_focus.get("title")) or "-"
    node_id = normalize_optional_text(current_focus.get("node_id")) or "-"
    plan_item_ref = normalize_optional_text(current_focus.get("plan_item_ref")) or "-"
    workflow_boundary = normalize_optional_text(current_focus.get("workflow_boundary")) or "-"
    task_id = normalize_optional_text(current_focus.get("task_id"))
    intent = normalize_optional_text(current_focus.get("intent")) or "-"
    acceptance = [str(item).strip() for item in (current_focus.get("acceptance") or []) if str(item).strip()]
    hotspot_keys = [str(item).strip() for item in (current_focus.get("hotspot_keys") or []) if str(item).strip()]
    depends_on = [str(item).strip() for item in (current_focus.get("depends_on") or []) if str(item).strip()]
    unlocks = [
        str(item.get("node_id") or "").strip()
        for item in (current_focus.get("unlocks") or [])
        if isinstance(item, dict) and str(item.get("node_id") or "").strip()
    ]
    lines = [
        "# Current Focus Plan Excerpt",
        "",
        f"- Plan: `{plan_id}`",
        f"- Source plan artifact: `{source_artifact or '-'}`",
        f"- Source plan ref: `{source_plan_ref or '-'}`",
        f"- Current node: `{node_id}`",
        f"- Current title: `{title}`",
        f"- Current plan ref: `{plan_item_ref}`",
        f"- Workflow boundary: `{workflow_boundary}`",
    ]
    if task_id:
        lines.append(f"- Current task: `{task_id}`")
    lines.extend(["", "## Intent", "", intent, "", "## Acceptance"])
    if acceptance:
        lines.extend([f"- {item}" for item in acceptance])
    else:
        lines.append("- No additional acceptance bullets were recorded in the planning IR.")
    lines.extend(["", "## Hotspots"])
    if hotspot_keys:
        lines.extend([f"- {item}" for item in hotspot_keys])
    else:
        lines.append("- No explicit hotspot keys recorded.")
    if depends_on:
        lines.extend(["", "## Depends On", *[f"- {item}" for item in depends_on]])
    if unlocks:
        lines.extend(["", "## Unlocks", *[f"- {item}" for item in unlocks]])
    return "\n".join(lines) + "\n"


def _task_dag_compact_packet_dispatch_artifact_paths(graph: dict[str, Any]) -> list[str]:
    dispatch_artifacts = graph.get("dispatch_artifacts")
    if not isinstance(dispatch_artifacts, dict):
        return []
    paths: list[str] = []
    for raw_value in dispatch_artifacts.values():
        value = normalize_optional_text(raw_value)
        if value is not None:
            paths.append(value)
    return paths


def _task_dag_compact_packet_worker_visible_dispatch_artifact_paths(
    graph: dict[str, Any],
    *,
    source_artifact_path: str | None,
) -> list[str]:
    source_path = normalize_optional_text(source_artifact_path)
    hidden_basenames = {
        "ait_directory_structure_decoupling_plan.md",
        "ait_module_ownership_map.md",
    }
    visible_paths: list[str] = []
    for raw_path in _task_dag_compact_packet_dispatch_artifact_paths(graph):
        path = normalize_optional_text(raw_path)
        if path is None:
            continue
        if source_path is not None and path == source_path:
            continue
        if Path(path).name in hidden_basenames:
            continue
        visible_paths.append(path)
    return visible_paths


def _task_dag_compact_packet_boundary_policy(
    *,
    packet_root_path: str,
    packet_root_manifest_path: str,
    execution_mode: str,
    final_remote_disposition_default: bool = False,
    graph_artifact_path: str | None = None,
    source_artifact_path: str | None = None,
    authoring_workspace_root: str | None = None,
    comparison_inputs_packaged: bool = False,
) -> dict[str, Any]:
    if execution_mode == "benchmark":
        allowed_file_hints = [
            "packet_root_manifest.json",
            "compact_worker_turn.txt",
        ]
        if comparison_inputs_packaged:
            allowed_file_hints.append("comparison_evidence.json")
        return {
            "policy_version": 2,
            "execution_mode": execution_mode,
            "policy_strength": "packet_root_workspace_with_allowlist_and_diagnostics",
            "packet_root_path": packet_root_path,
            "packet_root_manifest_path": packet_root_manifest_path,
            "allowed_path_prefixes": [packet_root_path],
            "allowed_file_hints": allowed_file_hints,
            "forbidden_command_patterns": list(TASK_DAG_COMPACT_PACKET_FORBIDDEN_COMMAND_PATTERNS),
            "max_command_count": DEFAULT_TASK_DAG_COMPACT_PACKET_MAX_COMMAND_COUNT,
            "missing_context_policy": "reply_with_missing_context_marker",
            "suggested_first_command": "cat packet_root_manifest.json",
        }
    allowed_file_hints = [
        packet_root_manifest_path,
        f"{packet_root_path}/compact_worker_turn.txt",
    ]
    if graph_artifact_path:
        allowed_file_hints.append(graph_artifact_path)
    if source_artifact_path:
        allowed_file_hints.append(source_artifact_path)
    max_command_count = (
        DEFAULT_TASK_DAG_IMPLEMENTATION_AUTO_LAND_PACKET_MAX_COMMAND_COUNT
        if final_remote_disposition_default
        else DEFAULT_TASK_DAG_IMPLEMENTATION_PACKET_MAX_COMMAND_COUNT
    )
    return {
        "policy_version": 2,
        "execution_mode": execution_mode,
        "policy_strength": (
            "authoring_workspace_with_gate_autonomy"
            if final_remote_disposition_default
            else "authoring_workspace_with_packet_entrypoint"
        ),
        "packet_root_path": packet_root_path,
        "packet_root_manifest_path": packet_root_manifest_path,
        "authoring_workspace_root": authoring_workspace_root,
        "allowed_path_prefixes": [".", packet_root_path],
        "allowed_file_hints": allowed_file_hints,
        "forbidden_command_patterns": [],
        "max_command_count": max_command_count,
        "missing_context_policy": "reply_with_missing_context_marker",
        "suggested_first_command": f"cat {packet_root_manifest_path}",
    }


def _task_dag_bootstrap_packet_root_workspace(
    ctx: RepoContext,
    *,
    packet_root: Path,
) -> dict[str, Any]:
    ait_dir = packet_root / ".ait"
    for path in (
        ait_dir,
        ait_dir / "objects" / "manifests",
        ait_dir / "objects" / "packs",
        ait_dir / "objects" / "tree-packs",
        ait_dir / "refs" / "lines",
        ait_dir / "workspace",
    ):
        path.mkdir(parents=True, exist_ok=True)
    config = load_config(ctx)
    bootstrap_config = {
        "repo_name": str(config.get("repo_name") or ctx.root.name),
        "default_line": str(config.get("default_line") or current_line(ctx)),
        "workspace_root": str(packet_root.resolve()),
    }
    config_path = ait_dir / "config.json"
    config_path.write_text(json.dumps(bootstrap_config, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    return {
        "workspace_root": str(packet_root.resolve()),
        "config_path": str(config_path.resolve()),
    }


def _task_dag_compact_packet_comparison_inputs_packaged(comparison_evidence: dict[str, Any] | None) -> bool:
    return bool(isinstance(comparison_evidence, dict) and comparison_evidence.get("available"))


def _task_dag_compact_packet_comparison_summary_lines(comparison_evidence: dict[str, Any] | None) -> list[str]:
    if not isinstance(comparison_evidence, dict) or not comparison_evidence.get("available"):
        return []
    totals = comparison_evidence.get("totals") if isinstance(comparison_evidence.get("totals"), dict) else {}
    summary_lines: list[str] = []
    for mode in ("git_linear", "ait_linear", "ait_dag"):
        total = totals.get(mode)
        if isinstance(total, int):
            summary_lines.append(f"- {mode}: total_tokens={total}")
    return summary_lines


def _task_dag_load_comparison_evidence(
    ctx: RepoContext,
    *,
    graph: dict[str, Any],
    report_path: Path,
    workload_id: str | None = None,
) -> dict[str, Any]:
    resolved_report = report_path if report_path.is_absolute() else ctx.root / report_path
    if not resolved_report.is_file():
        raise ValueError(f"Comparison evidence report not found: {report_path}")
    try:
        report_payload = json.loads(resolved_report.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        raise ValueError(f"Comparison evidence report is not valid JSON: {report_path}") from exc
    workloads = report_payload.get("workloads")
    if not isinstance(workloads, list) or not workloads:
        raise ValueError(f"Comparison evidence report has no workloads: {report_path}")
    selected_workload_id = normalize_optional_text(
        workload_id
        or graph.get("comparison_evidence_workload_id")
        or graph.get("benchmark_workload_id")
    )
    row: dict[str, Any] | None = None
    if selected_workload_id:
        for candidate in workloads:
            if not isinstance(candidate, dict):
                continue
            if normalize_optional_text(candidate.get("workload_id")) == selected_workload_id:
                row = candidate
                break
        if row is None:
            raise ValueError(
                f"Comparison evidence workload {selected_workload_id!r} was not found in report {report_path}."
            )
    elif len(workloads) == 1 and isinstance(workloads[0], dict):
        row = workloads[0]
    else:
        raise ValueError(
            "Comparison evidence report contains multiple workloads; pass --comparison-evidence-workload-id or "
            "set graph.comparison_evidence_workload_id."
        )
    baseline_mode = normalize_optional_text(row.get("baseline_mode")) or "git_linear"
    totals: dict[str, int] = {}
    comparisons = row.get("comparisons") if isinstance(row.get("comparisons"), list) else []
    for comparison in comparisons:
        if not isinstance(comparison, dict):
            continue
        comparison_mode = normalize_optional_text(comparison.get("mode"))
        if comparison_mode:
            candidate_total = comparison.get("candidate_median_total_tokens")
            if isinstance(candidate_total, int):
                totals[comparison_mode] = candidate_total
        baseline_total = comparison.get("baseline_median_total_tokens")
        if isinstance(baseline_total, int):
            totals.setdefault(baseline_mode, baseline_total)
    runs = row.get("runs") if isinstance(row.get("runs"), list) else []
    sanitized_runs: list[dict[str, Any]] = []
    for run in runs:
        if not isinstance(run, dict):
            continue
        mode = normalize_optional_text(run.get("mode"))
        total_tokens = run.get("total_tokens")
        if mode and isinstance(total_tokens, int):
            totals.setdefault(mode, total_tokens)
        sanitized_runs.append(
            {
                "mode": mode,
                "run_id": normalize_optional_text(run.get("run_id")),
                "quality": normalize_optional_text(run.get("quality")),
                "quality_passed": bool(run.get("quality_passed")),
                "prompt_tokens": run.get("prompt_tokens"),
                "completion_tokens": run.get("completion_tokens"),
                "total_tokens": total_tokens,
                "usage_provenance": run.get("usage_provenance"),
            }
        )
    return {
        "available": True,
        "source_report_path": _task_dag_relative_path(ctx, resolved_report),
        "source_benchmark_id": normalize_optional_text(report_payload.get("benchmark_id")),
        "workload_id": normalize_optional_text(row.get("workload_id")),
        "title": normalize_optional_text(row.get("title")),
        "baseline_mode": baseline_mode,
        "totals": totals,
        "comparisons": comparisons,
        "runs": sanitized_runs,
        "summary_lines": _task_dag_compact_packet_comparison_summary_lines({"available": True, "totals": totals}),
    }


def _task_dag_compact_packet_preferred_missing_context_reply(
    *,
    packet_available: bool,
    compare_text: str | None,
) -> str:
    if not packet_available:
        return (
            "missing_context: compact worker packet is graph-derived only and does not carry the planning-compiler "
            "artifact; request a refreshed packet before broadening scope."
        )
    if normalize_optional_text(compare_text):
        return (
            "missing_context: this compact worker packet does not include provider-measured comparison evidence; "
            "stop at the packet boundary and request packaged comparison totals instead of broadening scope."
        )
    return (
        "missing_context: the compact worker packet is insufficient for a grounded answer; stop at the packet "
        "boundary and request the missing inputs instead of broadening scope."
    )


def _task_dag_compact_packet_payload(
    compiler_surface: dict[str, Any],
    *,
    execution_mode: str,
) -> dict[str, Any]:
    preferred_key = "benchmark_packet" if execution_mode == "benchmark" else "execution_packet"
    packet = compiler_surface.get(preferred_key) if isinstance(compiler_surface.get(preferred_key), dict) else {}
    if packet:
        return packet
    fallback_key = "execution_packet" if preferred_key == "benchmark_packet" else "benchmark_packet"
    fallback = compiler_surface.get(fallback_key) if isinstance(compiler_surface.get(fallback_key), dict) else {}
    return fallback


def _task_dag_compact_packet_rewrite_packet_text(
    packet: dict[str, Any],
    *,
    source_artifact_path: str | None,
    source_artifact_hint: str | None,
) -> dict[str, Any]:
    if not packet:
        return {}
    source_path = normalize_optional_text(source_artifact_path)
    source_hint = normalize_optional_text(source_artifact_hint)
    if source_path is None or source_hint is None or source_path == source_hint:
        return packet
    rewritten = dict(packet)
    context_text = normalize_optional_text(rewritten.get("context_text"))
    if context_text is not None:
        rewritten["context_text"] = context_text.replace(source_path, source_hint)
    return rewritten


def _task_dag_compact_packet_change_focus_policy(compact_packet_surface: dict[str, Any]) -> dict[str, Any]:
    return (
        compact_packet_surface.get("change_focus_policy")
        if isinstance(compact_packet_surface.get("change_focus_policy"), dict)
        else {}
    )


def _task_dag_compact_packet_focus_queue(change_focus_policy: dict[str, Any]) -> list[dict[str, Any]]:
    return [entry for entry in (change_focus_policy.get("focus_queue") or []) if isinstance(entry, dict)]


def _task_dag_compact_packet_current_focus(
    compact_packet_surface: dict[str, Any],
    compiler_surface: dict[str, Any],
    *,
    graph: dict[str, Any],
) -> dict[str, Any] | None:
    change_focus_policy = _task_dag_compact_packet_change_focus_policy(compact_packet_surface)
    focus_queue = _task_dag_compact_packet_focus_queue(change_focus_policy)
    next_focus = change_focus_policy.get("next_focus") if isinstance(change_focus_policy.get("next_focus"), dict) else {}
    if not next_focus and focus_queue:
        next_focus = dict(focus_queue[0])
    node_id = normalize_optional_text(next_focus.get("node_id")) or normalize_optional_text(compact_packet_surface.get("next_focus_node_id"))
    if node_id is None:
        return None
    planning_ir = compiler_surface.get("planning_ir") if isinstance(compiler_surface.get("planning_ir"), dict) else {}
    work_item_index = {
        str(item.get("node_id") or "").strip(): item
        for item in planning_ir.get("work_items") or []
        if isinstance(item, dict) and str(item.get("node_id") or "").strip()
    }
    work_item = work_item_index.get(node_id, {})
    title = normalize_optional_text(next_focus.get("title")) or normalize_optional_text(work_item.get("title")) or node_id
    plan_item_ref = normalize_optional_text(next_focus.get("plan_item_ref")) or normalize_optional_text(work_item.get("plan_item_ref"))
    task_id = normalize_optional_text(next_focus.get("task_id")) or normalize_optional_text(compact_packet_surface.get("next_focus_task_id"))
    change_id = normalize_optional_text(next_focus.get("change_id")) or normalize_optional_text(compact_packet_surface.get("next_focus_change_id"))
    patchset_id = normalize_optional_text(next_focus.get("patchset_id"))
    state = normalize_optional_text(next_focus.get("state"))
    workflow_state = normalize_optional_text(next_focus.get("workflow_state")) or state
    workflow_boundary = normalize_optional_text(work_item.get("workflow_boundary"))
    intent = normalize_optional_text(work_item.get("intent"))
    acceptance = [str(item).strip() for item in (work_item.get("acceptance") or []) if str(item).strip()]
    hotspot_keys = [str(item).strip() for item in (work_item.get("hotspot_keys") or []) if str(item).strip()]
    depends_on = [str(item).strip() for item in (work_item.get("depends_on") or []) if str(item).strip()]
    unlocks: list[dict[str, str]] = []
    for edge in graph.get("edges") or []:
        if not isinstance(edge, dict):
            continue
        from_node = normalize_optional_text(edge.get("from") or edge.get("source"))
        to_node = normalize_optional_text(edge.get("to") or edge.get("target"))
        if from_node != node_id or to_node is None:
            continue
        target_item = work_item_index.get(to_node, {})
        unlocks.append(
            {
                "node_id": to_node,
                "title": normalize_optional_text(target_item.get("title")) or to_node,
            }
        )
    current_focus = {
        "node_id": node_id,
        "title": title,
        "plan_item_ref": plan_item_ref,
        "task_id": task_id,
        "change_id": change_id,
        "patchset_id": patchset_id,
        "state": state,
        "workflow_state": workflow_state,
        "workflow_boundary": workflow_boundary,
        "intent": intent,
        "acceptance": acceptance,
        "hotspot_keys": hotspot_keys,
        "depends_on": depends_on,
        "unlocks": unlocks,
    }
    if len(focus_queue) > 1:
        current_focus["ignore_non_active_nodes"] = True
    return current_focus


def _task_dag_compact_packet_current_focus_context(
    current_focus: dict[str, Any] | None,
    *,
    fallback_text: str,
) -> tuple[str, str]:
    if not isinstance(current_focus, dict):
        return "Packet context:", fallback_text
    node_id = normalize_optional_text(current_focus.get("node_id"))
    if node_id is None:
        return "Packet context:", fallback_text
    title = normalize_optional_text(current_focus.get("title"))
    task_id = normalize_optional_text(current_focus.get("task_id"))
    change_id = normalize_optional_text(current_focus.get("change_id"))
    workflow_state = normalize_optional_text(current_focus.get("workflow_state")) or normalize_optional_text(current_focus.get("state"))
    plan_item_ref = normalize_optional_text(current_focus.get("plan_item_ref"))
    workflow_boundary = normalize_optional_text(current_focus.get("workflow_boundary"))
    depends_on = [str(item).strip() for item in (current_focus.get("depends_on") or []) if str(item).strip()]
    unlocks = [str((item or {}).get("node_id") or "").strip() for item in (current_focus.get("unlocks") or []) if str((item or {}).get("node_id") or "").strip()]
    acceptance = [str(item).strip() for item in (current_focus.get("acceptance") or []) if str(item).strip()]
    hotspot_keys = [str(item).strip() for item in (current_focus.get("hotspot_keys") or []) if str(item).strip()]
    intent = normalize_optional_text(current_focus.get("intent"))
    lines = ["Current focus:"]
    lines.append(f"- node {node_id}" + (f" · {title}" if title else ""))
    identity_fields = []
    if task_id:
        identity_fields.append(f"task {task_id}")
    if change_id:
        identity_fields.append(f"change {change_id}")
    if workflow_state:
        identity_fields.append(f"state {workflow_state}")
    if identity_fields:
        lines.append("- " + " · ".join(identity_fields))
    boundary_fields = []
    if plan_item_ref:
        boundary_fields.append(f"plan ref {plan_item_ref}")
    if workflow_boundary:
        boundary_fields.append(f"boundary {workflow_boundary}")
    if boundary_fields:
        lines.append("- " + " · ".join(boundary_fields))
    if depends_on:
        lines.append("- depends on: " + ", ".join(depends_on))
    if unlocks:
        lines.append("- unlocks after completion: " + ", ".join(unlocks))
    if current_focus.get("ignore_non_active_nodes"):
        lines.append("- ignore non-active nodes until focus advances")
    if intent:
        lines.extend(["Intent:", intent])
    if acceptance:
        lines.append("Acceptance:")
        lines.extend(f"- {item}" for item in acceptance)
    if hotspot_keys:
        lines.append("Hotspots:")
        lines.extend(f"- {item}" for item in hotspot_keys)
    return "Current focus context:", "\n".join(lines)


def _task_dag_compact_packet_final_remote_disposition_lines(
    *,
    graph: dict[str, Any],
    graph_run_session_id: str | None,
) -> list[str]:
    graph_run_ref = graph_run_session_id or "<graph-run-session>"
    converged_output_node_ids = [node_id for node_id in _task_dag_converged_output_node_ids(graph) if node_id]
    final_gate_bundle = [
        str(value).strip().lower()
        for value in (
            (graph.get("execution_policy") if isinstance(graph.get("execution_policy"), dict) else {}).get("final_gate_bundle")
            or DEFAULT_TASK_DAG_FINAL_GATE_BUNDLE
        )
        if str(value).strip()
    ]
    lines = [
        "- This run ends at remote land; do not stop at the converged output unless a real blocker prevents completion.",
        f"- Graph-run session for lineage updates: `{graph_run_ref}`.",
        f"- Converged output nodes: `{', '.join(converged_output_node_ids) or '-'}`.",
        "- Keep execution-only lineage local-first, and do not skip honest local task/change lineage for real edits.",
        "- When the converged output is ready, reuse or publish its change lineage and carry it through `ait workflow land` before you reply.",
        f"- Required remote gate bundle for this run: `{', '.join(final_gate_bundle) or 'review,attestation,policy,land'}`.",
        "- If any lineage, gate, or land command blocks, stop there and report the exact failing command or missing prerequisite.",
    ]
    return lines


def _task_dag_compact_packet_turn_text(
    compact_packet_surface: dict[str, Any],
    compiler_surface: dict[str, Any],
    *,
    plan_id: str,
    graph: dict[str, Any],
    graph_artifact_path: str | None,
    graph_run_session_id: str | None = None,
    packet_root_policy: dict[str, Any],
    execution_mode: str,
    final_remote_disposition_default: bool = False,
    comparison_evidence: dict[str, Any] | None = None,
    source_artifact_path: str | None = None,
    source_artifact_hint: str | None = None,
) -> str:
    selected_packet = _task_dag_compact_packet_rewrite_packet_text(
        _task_dag_compact_packet_payload(compiler_surface, execution_mode=execution_mode),
        source_artifact_path=source_artifact_path,
        source_artifact_hint=source_artifact_hint,
    )
    compiler_input_bundle = (
        compiler_surface.get("compiler_input_bundle")
        if isinstance(compiler_surface.get("compiler_input_bundle"), dict)
        else {}
    )
    prompt_text = normalize_optional_text(selected_packet.get("prompt_text")) or "Use only this compact DAG packet."
    context_text = normalize_optional_text(selected_packet.get("context_text")) or "Compact DAG packet unavailable."
    compare_text = normalize_optional_text(compact_packet_surface.get("compare_turn_text"))
    packet_available = bool(compiler_input_bundle.get("available"))
    comparison_inputs_packaged = _task_dag_compact_packet_comparison_inputs_packaged(comparison_evidence)
    preferred_missing_context_reply = _task_dag_compact_packet_preferred_missing_context_reply(
        packet_available=packet_available,
        compare_text=compare_text,
    )
    allowed_file_hints = [str(row).strip() for row in packet_root_policy.get("allowed_file_hints") or [] if str(row).strip()]
    forbidden_patterns = [str(row).strip() for row in packet_root_policy.get("forbidden_command_patterns") or [] if str(row).strip()]
    max_command_count = int(packet_root_policy.get("max_command_count") or DEFAULT_TASK_DAG_COMPACT_PACKET_MAX_COMMAND_COUNT)
    suggested_first_command = normalize_optional_text(packet_root_policy.get("suggested_first_command")) or "cat packet_root_manifest.json"
    authoring_workspace_root = normalize_optional_text(packet_root_policy.get("authoring_workspace_root"))
    change_focus_policy = _task_dag_compact_packet_change_focus_policy(compact_packet_surface)
    focus_queue = _task_dag_compact_packet_focus_queue(change_focus_policy)
    current_focus = _task_dag_compact_packet_current_focus(
        compact_packet_surface,
        compiler_surface,
        graph=graph,
    )
    reviewable_focus = _task_dag_compact_packet_focus_is_reviewable_output(current_focus)
    context_heading, worker_context_text = _task_dag_compact_packet_current_focus_context(
        current_focus,
        fallback_text=context_text,
    )
    lines = ["Start here:"]
    if execution_mode == "benchmark":
        lines.extend(
            [
                f"- Read `{suggested_first_command}` first.",
                "- Stay inside the current packet-root workspace and use the packet context below as the authoritative scope.",
                "- If the packet is insufficient, reply with `missing_context:` instead of broadening scope.",
                f"- Keep the command footprint tight; target <= {max_command_count} shell/file probes before you answer.",
            ]
        )
        if comparison_inputs_packaged:
            lines.append(
                "- Use packaged comparison evidence if benchmark comparison is needed."
            )
        else:
            lines.append(
                f"- If measured comparison evidence is required and not packaged, reply exactly with: `{preferred_missing_context_reply}`"
            )
    else:
        lines.extend(
            [
                f"- Read `{suggested_first_command}` first.",
                "- Work in the resolved authoring workspace for implementation and verification; if the active focus already has a bound task worktree, use that instead of repo root.",
                "- Stay scoped to this graph and current focus; do not spawn additional DAG sessions or broad repo replay.",
                "- Execution-only focus with file edits: create or reuse the local change, run focused edits/tests, run `ait workspace status --json`, then run `ait snapshot create --message \"<focused slice>\"` before `ait task complete --local <task-id>`.",
                "- Execution-only focus with no scoped file edits: `ait task complete --local <task-id>` is allowed only for a true no-op / pure verification outcome.",
                "- End your reply with one `task_dag_local_progress={...}` line.",
                "- If the packet or workspace is insufficient, reply with `missing_context:` instead of inventing progress.",
                f"- Keep the command footprint disciplined; target <= {max_command_count} shell/file actions unless a grounded implementation path clearly needs more.",
            ]
        )
        if reviewable_focus:
            lines.extend(
                [
                    "- Reviewable-output focus: if promotion is still needed, run `ait workflow publish --task <task-id> --summary \"final output\" --target-line main`, then inspect or advance gates with `ait workflow land <change-id>` or `ait workflow land <change-id> --apply`.",
                    "- Keep one reviewable focus change active at a time, and publish its patchset before moving to the next reviewable focus.",
                    "- Report review, attestation, policy, and land status only after you actually run those gates.",
                ]
            )
        if authoring_workspace_root:
            lines.append(f"- Resolved authoring workspace root: `{authoring_workspace_root}`.")
        if final_remote_disposition_default and reviewable_focus:
            lines.extend(
                _task_dag_compact_packet_final_remote_disposition_lines(
                    graph=graph,
                    graph_run_session_id=graph_run_session_id,
                )
            )
    if allowed_file_hints:
        heading = "Allowed packet-root files:" if execution_mode == "benchmark" else "Suggested packet / graph entry files:"
        lines.extend(["", heading])
        lines.extend([f"- {name}" for name in allowed_file_hints])
    if forbidden_patterns:
        lines.extend(["", "Do not run commands like:"])
        lines.extend([f"- {pattern}" for pattern in forbidden_patterns])
    if execution_mode == "benchmark":
        lines.extend(
            [
                "",
                "Benchmark packet brief:",
                prompt_text,
            ]
        )
    lines.extend(
        [
            "",
            context_heading,
            worker_context_text,
        ]
    )
    comparison_summary_lines = _task_dag_compact_packet_comparison_summary_lines(comparison_evidence)
    if comparison_summary_lines:
        lines.extend(
            [
                "",
                "Packaged comparison evidence:",
                *comparison_summary_lines,
            ]
        )
        source_report_path = normalize_optional_text((comparison_evidence or {}).get("source_report_path"))
        workload_id = normalize_optional_text((comparison_evidence or {}).get("workload_id"))
        title = normalize_optional_text((comparison_evidence or {}).get("title"))
        if workload_id or title:
            lines.append(
                f"- workload: {workload_id or '-'} | title: {title or '-'}"
            )
        if source_report_path:
            lines.append(f"- source_report: {source_report_path}")
    if compare_text:
        lines.extend(
            [
                "",
                "Comparison note:",
                (
                    "Use only packaged comparison evidence from this worker surface. Do not launch extra sessions or "
                    "reopen repo benchmark files."
                    if comparison_inputs_packaged
                    else "Comparison inputs are not auto-expanded from this worker surface. If the packet requires external "
                    "provider-measured evidence, stop at the packet boundary and emit `missing_context:` instead of "
                    "launching extra sessions or reopening repo files."
                ),
                compare_text,
            ]
        )
    if execution_mode == "benchmark" and not comparison_inputs_packaged:
        lines.extend(
            [
                "",
                "Manifest fallback reply:",
                preferred_missing_context_reply,
            ]
        )
    if not packet_available:
        lines.extend(
            [
                "",
                "Packet status:",
                "The planning-compiler source artifact was unavailable, so this packet is graph-derived only. Stay within this reduced packet root and report any missing context explicitly.",
            ]
        )
    return "\n".join(lines)


def _task_dag_compact_packet_markdown(
    *,
    plan_id: str,
    graph: dict[str, Any],
    graph_run_session_id: str | None,
    compact_packet_surface: dict[str, Any],
    compiler_surface: dict[str, Any],
    packet_root_policy: dict[str, Any],
    turn_text: str,
    execution_mode: str,
    final_remote_disposition_default: bool = False,
    comparison_evidence: dict[str, Any] | None = None,
    source_artifact_path: str | None = None,
    source_artifact_hint: str | None = None,
) -> str:
    selected_packet = _task_dag_compact_packet_rewrite_packet_text(
        _task_dag_compact_packet_payload(compiler_surface, execution_mode=execution_mode),
        source_artifact_path=source_artifact_path,
        source_artifact_hint=source_artifact_hint,
    )
    compiler_input_bundle = (
        compiler_surface.get("compiler_input_bundle")
        if isinstance(compiler_surface.get("compiler_input_bundle"), dict)
        else {}
    )
    change_focus_policy = (
        _task_dag_compact_packet_change_focus_policy(compact_packet_surface)
    )
    focus_queue = _task_dag_compact_packet_focus_queue(change_focus_policy)
    compare_text = normalize_optional_text(compact_packet_surface.get("compare_turn_text"))
    comparison_inputs_packaged = _task_dag_compact_packet_comparison_inputs_packaged(comparison_evidence)
    preferred_missing_context_reply = _task_dag_compact_packet_preferred_missing_context_reply(
        packet_available=bool(compiler_input_bundle.get("available")),
        compare_text=compare_text,
    )
    current_focus = _task_dag_compact_packet_current_focus(
        compact_packet_surface,
        compiler_surface,
        graph=graph,
    )
    current_focus_heading, current_focus_text = _task_dag_compact_packet_current_focus_context(
        current_focus,
        fallback_text=str(selected_packet.get("context_text") or ""),
    )
    lines = [
        "# Task DAG Compact Packet",
        "",
        f"- Plan: `{plan_id}`",
        f"- Graph: `{str(graph.get('graph_id') or plan_id)}`",
        f"- Surface: `{compact_packet_surface.get('surface_id')}`",
        f"- Execution mode: `{execution_mode}`",
        f"- Final remote disposition default: `{'yes' if final_remote_disposition_default else 'no'}`",
        f"- Packet available: `{'yes' if compiler_input_bundle.get('available') else 'no'}`",
        f"- Fresh worker session: `{compact_packet_surface.get('fresh_worker_session')}`",
        f"- Physical fan-out: `{compact_packet_surface.get('physical_fanout')}`",
        "",
        "## Execution rules",
        "",
        "- use the supplied compact packet as the active execution scope",
        (
            "- keep work inside the packet-root boundary"
            if execution_mode == "benchmark"
            else "- keep repository work scoped to the current graph and resolved authoring workspace, using the bound task worktree when the active focus already has one"
        ),
        *(
            []
            if execution_mode == "benchmark"
            else [
                "- keep one reviewable focus change active at a time",
                "- publish the active focus patchset before mutating the next reviewable focus change",
            ]
        ),
        "",
        "## Packet-root boundary",
        "",
        f"- Packet root: `{packet_root_policy.get('packet_root_path')}`",
        f"- Allowed file hints: `{', '.join(str(row) for row in packet_root_policy.get('allowed_file_hints') or [])}`",
        f"- Primary entrypoint command: `{packet_root_policy.get('suggested_first_command')}`",
        f"- Comparison inputs packaged: `{'yes' if comparison_inputs_packaged else 'no'}`",
        f"- Max command count target: `{packet_root_policy.get('max_command_count')}`",
        f"- Forbidden probe patterns: `{', '.join(str(row) for row in packet_root_policy.get('forbidden_command_patterns') or [])}`",
        f"- Preferred `missing_context:` reply: `{preferred_missing_context_reply}`",
        "",
        "## Packet prompt",
        "",
        "```text",
        str(selected_packet.get("prompt_text") or ""),
        "```",
    ]
    if execution_mode != "benchmark" and current_focus_heading == "Current focus context:":
        lines.extend(
            [
                "",
                "## Worker current focus context",
                "",
                "```text",
                current_focus_text,
                "```",
            ]
        )
    lines.extend(
        [
        "",
        "## Packet context",
        "",
        "```text",
        str(selected_packet.get("context_text") or ""),
        "```",
        ]
    )
    if final_remote_disposition_default:
        lines.extend(
            [
                "",
                "## Final remote disposition",
                "",
                f"- graph-run session: `{graph_run_session_id or '-'}`",
                f"- converged output nodes: `{', '.join(_task_dag_converged_output_node_ids(graph)) or '-'}`",
                f"- required gates: `{', '.join(str(value) for value in ((graph.get('execution_policy') if isinstance(graph.get('execution_policy'), dict) else {}).get('final_gate_bundle') or DEFAULT_TASK_DAG_FINAL_GATE_BUNDLE))}`",
            ]
        )
    comparison_summary_lines = _task_dag_compact_packet_comparison_summary_lines(comparison_evidence)
    if comparison_summary_lines:
        lines.extend(
            [
                "",
                "## Packaged comparison evidence",
                "",
                *comparison_summary_lines,
            ]
        )
        workload_id = normalize_optional_text((comparison_evidence or {}).get("workload_id"))
        title = normalize_optional_text((comparison_evidence or {}).get("title"))
        source_report_path = normalize_optional_text((comparison_evidence or {}).get("source_report_path"))
        if workload_id or title:
            lines.append(f"- workload: `{workload_id or '-'}`")
            lines.append(f"- title: `{title or '-'}`")
        if source_report_path:
            lines.append(f"- source report: `{source_report_path}`")
    if focus_queue and execution_mode != "benchmark":
        lines.extend(
            [
                "",
                "## Reviewable focus queue",
                "",
            ]
        )
        for entry in focus_queue:
            if not isinstance(entry, dict):
                continue
            focus_unit = str(entry.get("focus_unit") or "node").strip()
            change_id = str(entry.get("change_id") or "").strip() or "-"
            node_id = str(entry.get("node_id") or "").strip() or "-"
            task_id = str(entry.get("task_id") or "").strip() or "-"
            state = str(entry.get("workflow_state") or entry.get("state") or "").strip() or "-"
            if focus_unit == "change" and change_id != "-":
                lines.append(f"- change `{change_id}` · node `{node_id}` · task `{task_id}` · state `{state}`")
            else:
                lines.append(f"- node `{node_id}` (change pending) · task `{task_id}` · state `{state}`")
    lines.extend(
        [
            "",
            "## Copyable turn",
            "",
            "```text",
            turn_text,
            "```",
        ]
    )
    return "\n".join(lines) + "\n"


def _task_dag_generate_compact_packet_artifacts(
    ctx: RepoContext,
    *,
    plan_id: str,
    graph: dict[str, Any],
    graph_path: Path,
    compact_packet_surface: dict[str, Any],
    graph_run_session_id: str | None = None,
    final_remote_disposition_default: bool = False,
    comparison_evidence: dict[str, Any] | None = None,
    authoring_workspace_root: str | None = None,
) -> dict[str, Any]:
    compiler_surface = build_task_dag_planning_compiler_surface(ctx.repo_root, graph, graph_path=graph_path)
    execution_mode = "benchmark" if comparison_evidence is not None else "implementation"
    repo_root = ctx.root.resolve()
    resolved_authoring_workspace_path = Path(authoring_workspace_root or ctx.root).expanduser().resolve()
    resolved_authoring_workspace_root = str(resolved_authoring_workspace_path)
    graph_slug = _task_dag_compact_packet_slug(str(graph.get("graph_id") or plan_id))
    output_dir = (
        repo_root
        / ".ait"
        / "generated"
        / "task_dag_compact_packets"
        / f"{graph_slug}-{int(time.time() * 1000)}"
    )
    output_dir.mkdir(parents=True, exist_ok=True)
    output_dir_relative = output_dir.relative_to(repo_root)
    packet_surface_path = output_dir / "planning_compiler_surface.json"
    packet_artifact_path = output_dir / "compact_packet.md"
    turn_artifact_path = output_dir / "compact_worker_turn.txt"
    packet_root_path = output_dir / "packet_root"
    packet_root_path.mkdir(parents=True, exist_ok=True)
    current_focus_excerpt_path = packet_root_path / "current_focus_plan_excerpt.md"
    packet_root_relative = _task_dag_relative_path(ctx, packet_root_path)
    packet_root_manifest_relative = _task_dag_relative_path(ctx, packet_root_path / "packet_root_manifest.json")
    comparison_inputs_packaged = _task_dag_compact_packet_comparison_inputs_packaged(comparison_evidence)
    compiler_input_bundle = (
        compiler_surface.get("compiler_input_bundle")
        if isinstance(compiler_surface.get("compiler_input_bundle"), dict)
        else {}
    )
    source_artifact_path = normalize_optional_text(compiler_input_bundle.get("artifact_path"))
    graph_artifact_hint = _task_dag_relative_path(ctx, graph_path)
    source_artifact_hint = source_artifact_path
    current_focus_excerpt_hint: str | None = None
    bridge_specs: list[tuple[Path, str]] = []
    authoring_context_bridge_hints: list[str] = []
    current_focus = (
        _task_dag_compact_packet_current_focus(
            compact_packet_surface,
            compiler_surface,
            graph=graph,
        )
        if execution_mode == "implementation"
        else None
    )
    if execution_mode == "implementation" and current_focus is not None:
        current_focus_excerpt_path.write_text(
            _task_dag_compact_packet_current_focus_excerpt_text(
                plan_id=plan_id,
                graph=graph,
                current_focus=current_focus,
                source_artifact_path=source_artifact_path,
            ),
            encoding="utf-8",
        )
        current_focus_excerpt_hint = _task_dag_relative_path(ctx, current_focus_excerpt_path)
        source_artifact_hint = current_focus_excerpt_hint
    if execution_mode == "implementation" and resolved_authoring_workspace_path != repo_root:
        bridged_hints: set[str] = set()
        graph_bridge_specs, graph_bridge_hints = _task_dag_compact_packet_bridge_files(
            ctx,
            output_dir_relative=output_dir_relative,
            repo_relative_paths=[graph_artifact_hint],
            seen_hints=bridged_hints,
        )
        if graph_bridge_hints:
            graph_artifact_hint = graph_bridge_hints[0]
            bridge_specs.extend(graph_bridge_specs)
        source_bridge_specs, source_bridge_hints = _task_dag_compact_packet_bridge_files(
            ctx,
            output_dir_relative=output_dir_relative,
            repo_relative_paths=(
                []
                if current_focus_excerpt_hint is not None
                else [source_artifact_path] if source_artifact_path else []
            ),
            seen_hints=bridged_hints,
        )
        if source_bridge_hints:
            source_artifact_hint = source_bridge_hints[0]
            bridge_specs.extend(source_bridge_specs)
        dispatch_bridge_specs, dispatch_bridge_hints = _task_dag_compact_packet_bridge_files(
            ctx,
            output_dir_relative=output_dir_relative,
            repo_relative_paths=_task_dag_compact_packet_worker_visible_dispatch_artifact_paths(
                graph,
                source_artifact_path=source_artifact_path,
            ),
            seen_hints=bridged_hints,
        )
        bridge_specs.extend(dispatch_bridge_specs)
        authoring_context_bridge_hints.extend(dispatch_bridge_hints)
    packet_root_policy = _task_dag_compact_packet_boundary_policy(
        packet_root_path=packet_root_relative,
        packet_root_manifest_path=packet_root_manifest_relative,
        execution_mode=execution_mode,
        final_remote_disposition_default=final_remote_disposition_default,
        graph_artifact_path=graph_artifact_hint,
        source_artifact_path=source_artifact_hint,
        authoring_workspace_root=resolved_authoring_workspace_root,
        comparison_inputs_packaged=comparison_inputs_packaged,
    )
    if authoring_context_bridge_hints:
        allowed_file_hints = [str(row).strip() for row in packet_root_policy.get("allowed_file_hints") or [] if str(row).strip()]
        for hint in authoring_context_bridge_hints:
            if hint not in allowed_file_hints:
                allowed_file_hints.append(hint)
        packet_root_policy["allowed_file_hints"] = allowed_file_hints
    worker_context_scope = "current_focus" if current_focus is not None else "packet"
    packet_artifact_relative = _task_dag_relative_path(ctx, packet_artifact_path)
    turn_text = _task_dag_compact_packet_turn_text(
        compact_packet_surface,
        compiler_surface,
        plan_id=plan_id,
        graph=graph,
        graph_artifact_path=graph_artifact_hint,
        graph_run_session_id=graph_run_session_id,
        packet_root_policy=packet_root_policy,
        execution_mode=execution_mode,
        final_remote_disposition_default=final_remote_disposition_default,
        comparison_evidence=comparison_evidence,
        source_artifact_path=source_artifact_path,
        source_artifact_hint=source_artifact_hint,
    )
    compiler_surface_text = json.dumps(compiler_surface, indent=2, ensure_ascii=False) + "\n"
    packet_surface_path.write_text(compiler_surface_text, encoding="utf-8")
    packet_markdown = _task_dag_compact_packet_markdown(
        plan_id=plan_id,
        graph=graph,
        graph_run_session_id=graph_run_session_id,
        compact_packet_surface=compact_packet_surface,
        compiler_surface=compiler_surface,
        packet_root_policy=packet_root_policy,
        turn_text=turn_text,
        execution_mode=execution_mode,
        final_remote_disposition_default=final_remote_disposition_default,
        comparison_evidence=comparison_evidence,
        source_artifact_path=source_artifact_path,
        source_artifact_hint=source_artifact_hint,
    )
    packet_artifact_path.write_text(
        packet_markdown,
        encoding="utf-8",
    )
    turn_artifact_path.write_text(turn_text + "\n", encoding="utf-8")
    packet_root_workspace = _task_dag_bootstrap_packet_root_workspace(ctx, packet_root=packet_root_path)
    packet_root_turn_path = packet_root_path / "compact_worker_turn.txt"
    packet_root_manifest_path = packet_root_path / "packet_root_manifest.json"
    packet_root_turn_path.write_text(turn_text + "\n", encoding="utf-8")
    selected_packet = _task_dag_compact_packet_payload(compiler_surface, execution_mode=execution_mode)
    compare_text = normalize_optional_text(compact_packet_surface.get("compare_turn_text"))
    preferred_missing_context_reply = _task_dag_compact_packet_preferred_missing_context_reply(
        packet_available=bool(compiler_input_bundle.get("available")),
        compare_text=compare_text,
    )
    comparison_evidence_artifact_path = output_dir / "comparison_evidence.json"
    packet_root_comparison_evidence_path = packet_root_path / "comparison_evidence.json"
    comparison_evidence_relative = None
    if comparison_inputs_packaged:
        evidence_text = json.dumps(comparison_evidence, indent=2, ensure_ascii=False) + "\n"
        comparison_evidence_artifact_path.write_text(evidence_text, encoding="utf-8")
        packet_root_comparison_evidence_path.write_text(evidence_text, encoding="utf-8")
        comparison_evidence_relative = _task_dag_relative_path(ctx, comparison_evidence_artifact_path)
    packet_root_manifest = {
        "schema_version": 1,
        "plan_id": plan_id,
        "graph_id": str(graph.get("graph_id") or plan_id),
        "execution_mode": execution_mode,
        "workspace_root": packet_root_workspace.get("workspace_root"),
        "authoring_workspace_root": resolved_authoring_workspace_root,
        "allowed_path_prefixes": packet_root_policy.get("allowed_path_prefixes") or [],
        "allowed_file_hints": packet_root_policy.get("allowed_file_hints") or [],
        "max_command_count": packet_root_policy.get("max_command_count"),
        "missing_context_policy": packet_root_policy.get("missing_context_policy"),
        "primary_entrypoint_file": "packet_root_manifest.json",
        "secondary_context_files": [
            "compact_worker_turn.txt",
            *(["current_focus_plan_excerpt.md"] if current_focus_excerpt_hint else []),
            *(["comparison_evidence.json"] if comparison_inputs_packaged else []),
        ],
        "suggested_first_command": packet_root_policy.get("suggested_first_command"),
    }
    forbidden_command_patterns = packet_root_policy.get("forbidden_command_patterns") or []
    if forbidden_command_patterns:
        packet_root_manifest["forbidden_command_patterns"] = forbidden_command_patterns
    if current_focus is not None:
        packet_root_manifest["current_focus"] = current_focus
    if comparison_inputs_packaged:
        packet_root_manifest["comparison_evidence_file"] = "comparison_evidence.json"
    packet_root_manifest_path.write_text(
        json.dumps(packet_root_manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    for source_path, relative_bridge_path in bridge_specs:
        bridge_path = repo_root / relative_bridge_path
        bridge_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source_path, bridge_path)
    if execution_mode == "implementation" and resolved_authoring_workspace_path != repo_root:
        authoring_output_dir = resolved_authoring_workspace_path / output_dir_relative
        authoring_output_dir.parent.mkdir(parents=True, exist_ok=True)
        same_output_tree = False
        try:
            same_output_tree = authoring_output_dir.exists() and authoring_output_dir.samefile(output_dir)
        except OSError:
            same_output_tree = False
        if not same_output_tree:
            shutil.copytree(output_dir, authoring_output_dir, dirs_exist_ok=True)
            compact_packet_copy = authoring_output_dir / "compact_packet.md"
            if compact_packet_copy.exists():
                compact_packet_copy.unlink()
    return {
        "packet_available": bool(compiler_input_bundle.get("available")),
        "packet_artifact_path": packet_artifact_relative,
        "surface_artifact_path": _task_dag_relative_path(ctx, packet_surface_path),
        "turn_artifact_path": _task_dag_relative_path(ctx, turn_artifact_path),
        "packet_root_path": packet_root_relative,
        "packet_root_manifest_path": _task_dag_relative_path(ctx, packet_root_manifest_path),
        "packet_root_workspace_root": packet_root_workspace.get("workspace_root"),
        "worker_workspace_root": resolved_authoring_workspace_root if execution_mode == "implementation" else packet_root_workspace.get("workspace_root"),
        "worker_repo_root": resolved_authoring_workspace_root if execution_mode == "implementation" else packet_root_workspace.get("workspace_root"),
        "packet_root_policy": packet_root_policy,
        "execution_mode": execution_mode,
        "final_remote_disposition_default": final_remote_disposition_default,
        "comparison_inputs_packaged": comparison_inputs_packaged,
        "comparison_evidence_artifact_path": comparison_evidence_relative,
        "turn_text": turn_text,
        "packet_prompt_digest": selected_packet.get("prompt_digest"),
        "packet_context_digest": selected_packet.get("context_digest"),
        "worker_context_scope": worker_context_scope,
        "current_focus": current_focus,
        "benchmark_packet_mode": selected_packet.get("mode"),
        "compiler_surface": compiler_surface,
    }
