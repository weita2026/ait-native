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
TASK_DAG_AUTHORING_WORKSPACE_RUNTIME_BOOTSTRAP_PATH = "ait-dag.md"


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
            "Please compress this DAG into a worker-only compact `ait_dag` packet, then execute it in one fresh worker "
            "session while keeping execution-only work local-first, then carry the converged reviewable output through "
            "the final remote review/attestation/policy/land gates without a coordinator handoff. Do not use physical "
            "fan-out, coordinator+worker end-to-end, or repo-wide replay beyond the packet boundary unless the packet "
            "explicitly broadens it."
            + focus_clause
        )
    else:
        copyable_turn_text = (
            "Please compress this DAG into a worker-only compact `ait_dag` packet, then execute it in one fresh worker "
            "session while keeping execution-only work local-first, then carry the converged reviewable output through "
            "one explicit local land on the target line before you stop. After that local land, the final converged output "
            "may later remote-promote through `ait workflow land --all-completed-local --remote <name>`. Do not use physical "
            "fan-out, coordinator+worker end-to-end, or repo-wide replay beyond the packet boundary unless the packet "
            "explicitly broadens it."
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


def _task_dag_compact_packet_bridge_text_artifacts(
    *,
    output_dir_relative: Path,
    text_artifacts: dict[str, str],
    seen_hints: set[str] | None = None,
) -> tuple[list[tuple[str, str]], list[str]]:
    seen = seen_hints if seen_hints is not None else set()
    bridge_specs: list[tuple[str, str]] = []
    bridge_hints: list[str] = []
    for relative_repo_path, body in text_artifacts.items():
        relative_value = normalize_optional_text(relative_repo_path)
        if relative_value is None:
            continue
        hint = _task_dag_compact_packet_bridge_relative_path(output_dir_relative, relative_value)
        if hint in seen:
            continue
        seen.add(hint)
        bridge_specs.append((hint, body))
        bridge_hints.append(hint)
    return bridge_specs, bridge_hints


def _task_dag_compact_packet_runtime_bootstrap_markdown() -> str:
    return (
        "# ait-dag\n\n"
        "This runtime-only helper applies only to worker-only compact `ait_dag` packets.\n"
        "Do not treat it as repo-root plan Markdown.\n\n"
        "## Startup order\n\n"
        "1. Read `packet_root_manifest.json` first.\n"
        "2. Use the packet entry files plus the allowed `authoring_workspace_context` inputs before broadening scope.\n"
        "3. Read `compact_worker_turn.txt` for the current task ids, focus queue, and gate expectations.\n"
        "4. Stay inside the resolved authoring workspace unless the packet explicitly authorizes something narrower or broader.\n\n"
        "## Stable command playbook\n\n"
        "- Start with `cat packet_root_manifest.json`.\n"
        "- Use targeted file reads such as `sed -n '<start>,<end>p' <file>` or `cat <specific-file>` only for files named by the manifest, turn text, or task-scoped artifacts.\n"
        "- Reopen `compact_packet.md` only when the current step truly needs the packet prompt/context again.\n"
        "- Treat `execution_only` as local-first workflow scope, not as permission to skip honest local lineage after mutating scoped files.\n"
        "- If the active execution-only focus stays a true no-op / pure verification slice with no scoped file edits, finish honestly with `ait task complete <task-id>`.\n"
        "- If the active execution-only focus needs scoped code/test/file edits and no local change exists yet, create or reuse honest local change lineage first, for example `ait change create --task <task-id> --title \"<focused slice>\" --local`.\n"
        "- Use `ait task complete <task-id>` only after the scoped local implementation/verification and any needed local change/snapshot lineage honestly reflect the completed outcome.\n"
        "- When a reviewable change already exists or gets created, inspect gate state with `ait workflow land <change-id>` and advance it with `ait workflow land <change-id> --apply`.\n"
        "- Only if the turn text says the final reviewable output still needs promotion, use `ait workflow publish --task <task-id> --summary \"final output\" --target-line main` before continuing with `ait workflow land ...`.\n"
        "- Run only focused verification commands that match the active change, such as `python3 -m pytest <scoped-target>`; the repository default already applies the three-worker xdist loadfile contract unless you intentionally override it.\n\n"
        "## Anti-exploration rules\n\n"
        "- Do not browse the repository just to learn general context.\n"
        "- Do not run broad discovery commands such as repo-wide `find`, `rg`, `fd`, or repeated directory walks unless the packet explicitly requires one precise scoped path.\n"
        "- Do not reopen the same packet or sprint files repeatedly once the needed fact is already known.\n"
        "- Prefer the smallest next command that advances the active focus over collecting extra context \"just in case\".\n"
        "- Prefer direct task/change workflow commands over unrelated repository inspection.\n"
        "- If a needed path, id, or prerequisite is missing, stop and reply with `missing_context: ...` or report the exact blocking command instead of broadening scope.\n\n"
        "## Worker rules\n\n"
        "- Keep exactly one reviewable focus change active at a time.\n"
        "- Keep execution-only DAG work local-first until the converged output reaches its real review / attestation / policy / land boundary.\n"
        "- Do not treat `execution_only` as permission to skip local change lineage after scoped code/test/file edits.\n"
        "- Do not claim a gate passed unless you actually ran that gate.\n"
        "- If required context is still missing, reply with `missing_context: ...` instead of inventing progress.\n"
        "- End execution replies with one `task_dag_local_progress={...}` line that reports the current node outcome.\n"
    )


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
            "compact_packet.md",
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
        f"{packet_root_path}/compact_packet.md",
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
        "- Full final-remote-disposition mode is enabled for this run. Do not stop at the converged output gate bundle or hand control back for a coordinator unless a real blocker prevents completion.",
        f"- Graph-run session for lineage updates: `{graph_run_ref}`.",
        f"- Converged output nodes for this graph: `{', '.join(converged_output_node_ids) or '-'}`.",
        "- If a node still needs task/change lineage, create or reuse it inside this same worker session; do not hand control back for per-node bootstrap or physical fan-out.",
        "- For execution-only nodes, keep any task/change/snapshot/local-land lineage local-first. A true no-op / pure verification node may close with `ait task complete <task-id>`, but scoped code/test/file edits must not skip honest local change lineage; if no local change exists yet, create or reuse it first (for example `ait change create --task <task-id> --title \"<focused slice>\" --local`). Close the task only once that local lineage and verification honestly reflect the completed outcome.",
        "- When a converged reviewable-output node is satisfied, reuse its change lineage or promote the final output with `ait workflow publish --task <task-id> --summary \"final output\" --target-line main`, then use `ait workflow land <change-id>` / `ait workflow land <change-id> --apply` to carry that change through remote gates before you reply.",
        f"- Required remote gate bundle for this run: `{', '.join(final_gate_bundle) or 'review,attestation,policy,land'}`.",
        "- Before replying, prefer `ait workflow land <change-id>` to inspect the current gate state and `ait workflow land <change-id> --apply` to advance the remote-land path. Fall back to explicit low-level commands only for exception or recovery paths such as manual attestation backfill.",
        "- If any lineage, gate, or land command blocks, stop there and report the exact failing command or missing prerequisite instead of claiming remote land.",
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
    change_focus_policy = (
        compact_packet_surface.get("change_focus_policy")
        if isinstance(compact_packet_surface.get("change_focus_policy"), dict)
        else {}
    )
    focus_queue = change_focus_policy.get("focus_queue") if isinstance(change_focus_policy.get("focus_queue"), list) else []
    lines = ["Execute this DAG as a worker-only compact `ait_dag` packet.", "", "Hard constraints:", "- Use one fresh worker session."]
    if execution_mode == "benchmark":
        lines.extend(
            [
                "- Stay inside the current packet-root workspace for every shell or file action.",
                "- Do not use physical fan-out.",
                "- Do not use coordinator+worker end-to-end.",
                "- Treat the inline packet prompt/context below as authoritative; do not reopen packet files just to restate them.",
                "- Treat the packet as self-contained; do not reopen repository plans, graphs, or benchmark docs by path.",
                "- Do not replay the whole repository beyond this packet root.",
                f"- If you inspect packet-root files at all, start with `{suggested_first_command}`; that manifest is the primary entrypoint and is usually enough.",
                "- Do not inspect `planning_compiler_surface.json` from this worker surface; it is outside the packet-root probe budget.",
                "- If the packet is insufficient, reply with `missing_context:` instead of silently broadening scope.",
                "- Stop immediately after the smallest probe set that justifies either a grounded answer or a `missing_context:` reply.",
                f"- Keep the command footprint tight; target <= {max_command_count} shell/file probes before you answer.",
            ]
        )
        if comparison_inputs_packaged:
            lines.append(
                "- If benchmark comparison is needed, use the packaged provider-measured totals below instead of broadening scope."
            )
        else:
            lines.append(
                f"- If the manifest says comparison inputs are not packaged and the task needs measured comparison, reply exactly with: `{preferred_missing_context_reply}`"
            )
    else:
        lines.extend(
            [
                "- Stay inside the resolved authoring workspace for shell and file actions; when implementation-mode focus lineage already has a bound task worktree, that authoring workspace is the bound task worktree rather than repo root.",
                "- Do not use physical fan-out.",
                "- Do not use coordinator+worker end-to-end.",
                "- Treat the inline packet prompt/context below as authoritative planning context; reopen repository files only as needed to implement or verify the scoped DAG work.",
                "- Keep the worker scoped to this graph and the resolved authoring workspace; do not jump to unrelated plans or repo-wide replay.",
                f"- If you inspect the packet bundle, start with `{suggested_first_command}`; that manifest is the primary entrypoint.",
                "- Keep exactly one reviewable focus change active at a time.",
                "- When the active focus becomes reviewable, cut and publish its patchset before mutating the next reviewable focus change.",
                "- Do not accumulate one shared dirty diff across multiple reviewable change boundaries.",
                "- Do not claim review, attestation, policy, or land passed unless you actually run those gates.",
                "- For the current execution-only focus, finish with explicit local-first status: true no-op / pure verification work may close with `ait task complete <task-id>`, but scoped code/test/file edits must not skip honest local change lineage. If no local change exists yet, create or reuse it before task completion; otherwise stop on the exact blocker that prevented completion.",
                "- When the packet bundles `authoring_workspace_context/...` copies of the source sprint or task-graph artifacts, prefer those packet-scoped copies over repo-root `docs/sprints/...` lookup.",
                "- End your reply with one line that starts `task_dag_local_progress=` followed by compact JSON such as `{\"node_id\":\"A\",\"status\":\"running\",\"summary\":\"local edits started\"}` or `{\"node_id\":\"A\",\"status\":\"completed\",\"summary\":\"lane done\",\"tests\":[\"pytest ...\"]}`.",
                "- If the packet or workspace is insufficient, reply with `missing_context:` instead of inventing progress.",
                f"- Keep the command footprint disciplined; target <= {max_command_count} shell/file actions unless a grounded implementation path clearly needs more.",
            ]
        )
        if authoring_workspace_root:
            lines.append(f"- Resolved authoring workspace root: `{authoring_workspace_root}`.")
        if final_remote_disposition_default:
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
    if focus_queue and execution_mode != "benchmark":
        lines.extend(["", "Reviewable focus queue:"])
        for entry in focus_queue:
            if not isinstance(entry, dict):
                continue
            focus_unit = str(entry.get("focus_unit") or "node").strip()
            change_id = str(entry.get("change_id") or "").strip() or "-"
            node_id = str(entry.get("node_id") or "").strip() or "-"
            task_id = str(entry.get("task_id") or "").strip() or "-"
            state = str(entry.get("workflow_state") or entry.get("state") or "").strip() or "-"
            if focus_unit == "change" and change_id != "-":
                lines.append(f"- change {change_id} · node {node_id} · task {task_id} · state {state}")
            else:
                lines.append(f"- node {node_id} (change pending) · task {task_id} · state {state}")
    if forbidden_patterns:
        lines.extend(["", "Do not run commands like:"])
        lines.extend([f"- {pattern}" for pattern in forbidden_patterns])
    lines.extend(
        [
            "",
            "Packet prompt:",
            prompt_text,
            "",
            "Packet context:",
            context_text,
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
        compact_packet_surface.get("change_focus_policy")
        if isinstance(compact_packet_surface.get("change_focus_policy"), dict)
        else {}
    )
    focus_queue = change_focus_policy.get("focus_queue") if isinstance(change_focus_policy.get("focus_queue"), list) else []
    compare_text = normalize_optional_text(compact_packet_surface.get("compare_turn_text"))
    comparison_inputs_packaged = _task_dag_compact_packet_comparison_inputs_packaged(comparison_evidence)
    preferred_missing_context_reply = _task_dag_compact_packet_preferred_missing_context_reply(
        packet_available=bool(compiler_input_bundle.get("available")),
        compare_text=compare_text,
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
        "- worker-only compact `ait_dag` packet",
        "- one fresh worker session",
        "- no physical fan-out",
        "- no coordinator+worker end-to-end",
        (
            "- no repo-wide replay beyond the packet-root boundary"
            if execution_mode == "benchmark"
            else "- keep repository work scoped to the current graph and resolved authoring workspace, using the bound task worktree when the active focus already has one"
        ),
        *(
            []
            if execution_mode == "benchmark"
            else [
                "- keep exactly one reviewable focus change active at a time",
                "- cut and publish the active focus patchset before mutating the next reviewable focus change",
                "- do not accumulate one shared dirty diff across multiple reviewable change boundaries",
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
        "",
        "## Packet context",
        "",
        "```text",
        str(selected_packet.get("context_text") or ""),
        "```",
    ]
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
    bridge_specs: list[tuple[Path, str]] = []
    bridge_text_specs: list[tuple[str, str]] = []
    authoring_context_bridge_hints: list[str] = []
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
            repo_relative_paths=[source_artifact_path] if source_artifact_path else [],
            seen_hints=bridged_hints,
        )
        if source_bridge_hints:
            source_artifact_hint = source_bridge_hints[0]
            bridge_specs.extend(source_bridge_specs)
        runtime_bridge_specs, runtime_bridge_hints = _task_dag_compact_packet_bridge_text_artifacts(
            output_dir_relative=output_dir_relative,
            text_artifacts={
                TASK_DAG_AUTHORING_WORKSPACE_RUNTIME_BOOTSTRAP_PATH: _task_dag_compact_packet_runtime_bootstrap_markdown(),
            },
            seen_hints=bridged_hints,
        )
        bridge_text_specs.extend(runtime_bridge_specs)
        authoring_context_bridge_hints.extend(runtime_bridge_hints)
        dispatch_bridge_specs, dispatch_bridge_hints = _task_dag_compact_packet_bridge_files(
            ctx,
            output_dir_relative=output_dir_relative,
            repo_relative_paths=_task_dag_compact_packet_dispatch_artifact_paths(graph),
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
    packet_root_packet_path = packet_root_path / "compact_packet.md"
    packet_root_turn_path = packet_root_path / "compact_worker_turn.txt"
    packet_root_manifest_path = packet_root_path / "packet_root_manifest.json"
    packet_root_packet_path.write_text(packet_markdown, encoding="utf-8")
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
        "surface_id": compact_packet_surface.get("surface_id"),
        "execution_mode": execution_mode,
        "final_remote_disposition_default": final_remote_disposition_default,
        "packet_available": bool(compiler_input_bundle.get("available")),
        "packet_root_path": packet_root_relative,
        "workspace_root": packet_root_workspace.get("workspace_root"),
        "authoring_workspace_root": resolved_authoring_workspace_root,
        "authoring_repo_root": str(ctx.repo_root.resolve()),
        "allowed_path_prefixes": packet_root_policy.get("allowed_path_prefixes") or [],
        "allowed_file_hints": packet_root_policy.get("allowed_file_hints") or [],
        "forbidden_command_patterns": packet_root_policy.get("forbidden_command_patterns") or [],
        "max_command_count": packet_root_policy.get("max_command_count"),
        "missing_context_policy": packet_root_policy.get("missing_context_policy"),
        "primary_entrypoint_file": "packet_root_manifest.json",
        "secondary_context_files": [
            "compact_packet.md",
            "compact_worker_turn.txt",
            *(["comparison_evidence.json"] if comparison_inputs_packaged else []),
        ],
        "turn_contains_packet_context": True,
        "suggested_first_command": packet_root_policy.get("suggested_first_command"),
        "graph_run_session_id": graph_run_session_id,
        "converged_output_node_ids": _task_dag_converged_output_node_ids(graph),
        "final_gate_bundle": list(
            (
                (graph.get("execution_policy") if isinstance(graph.get("execution_policy"), dict) else {}).get("final_gate_bundle")
                or DEFAULT_TASK_DAG_FINAL_GATE_BUNDLE
            )
        ),
        "comparison_inputs_packaged": comparison_inputs_packaged,
        "comparison_instruction": compare_text,
        "preferred_missing_context_reply": preferred_missing_context_reply,
        "comparison_evidence_file": "comparison_evidence.json" if comparison_inputs_packaged else None,
        "comparison_evidence_summary": _task_dag_compact_packet_comparison_summary_lines(comparison_evidence),
        "packet_prompt_digest": selected_packet.get("prompt_digest"),
        "packet_context_digest": selected_packet.get("context_digest"),
        "change_focus_policy": compact_packet_surface.get("change_focus_policy") or {},
        "source_surface_artifact_path": _task_dag_relative_path(ctx, packet_surface_path),
        "source_packet_artifact_path": packet_artifact_relative,
        "source_turn_artifact_path": _task_dag_relative_path(ctx, turn_artifact_path),
    }
    packet_root_manifest_path.write_text(
        json.dumps(packet_root_manifest, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    for source_path, relative_bridge_path in bridge_specs:
        bridge_path = repo_root / relative_bridge_path
        bridge_path.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source_path, bridge_path)
    for relative_bridge_path, body in bridge_text_specs:
        bridge_path = repo_root / relative_bridge_path
        bridge_path.parent.mkdir(parents=True, exist_ok=True)
        bridge_path.write_text(body if body.endswith("\n") else body + "\n", encoding="utf-8")
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
        "benchmark_packet_mode": selected_packet.get("mode"),
        "compiler_surface": compiler_surface,
    }
