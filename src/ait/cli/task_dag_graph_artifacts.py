from __future__ import annotations

from pathlib import Path
from typing import Any

from ..plan_graph import load_task_graph
from ..store import RepoContext
from ..task_dag_conversation import discover_task_dag_graph_paths
from .task_dag_runtime_helpers import _task_dag_relative_path

_TASK_DAG_CANONICAL_PATH_HINT = "Prefer docs/sprints/<name>.task_graph.json for active artifacts."


def _task_dag_graph_path_family(ctx: RepoContext, path: Path) -> str:
    try:
        relative = path.resolve().relative_to(ctx.root.resolve())
    except ValueError:
        return ""
    parts = relative.parts
    if len(parts) >= 2 and parts[0] == "docs":
        return f"docs/{parts[1]}"
    return "docs"


def _load_task_dag_graph_for_plan(ctx: RepoContext, plan_id: str, graph_path: Path | None) -> tuple[dict[str, Any], Path]:
    if graph_path is not None:
        resolved_path = graph_path if graph_path.is_absolute() else ctx.root / graph_path
        graph = load_task_graph(resolved_path)
        source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
        graph_plan_id = source_plan.get("plan_id")
        if graph_plan_id != plan_id:
            raise ValueError(f"Task graph {resolved_path} belongs to plan {graph_plan_id!r}, not {plan_id!r}.")
        return graph, resolved_path

    matches: list[tuple[dict[str, Any], Path]] = []
    errors: list[str] = []
    for candidate in discover_task_dag_graph_paths(ctx.root):
        try:
            graph = load_task_graph(candidate)
        except ValueError as exc:
            errors.append(f"{candidate}: {exc}")
            continue
        source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
        if source_plan.get("plan_id") == plan_id:
            matches.append((graph, candidate))

    if not matches:
        detail = f" Validation errors: {'; '.join(errors[:3])}" if errors else ""
        raise ValueError(
            f"No task graph JSON artifact found for plan {plan_id!r}. "
            f"Use --from-json <path>. {_TASK_DAG_CANONICAL_PATH_HINT}{detail}"
        )

    preferred_graph, preferred_path = matches[0]
    preferred_family = _task_dag_graph_path_family(ctx, preferred_path)
    preferred_group = [
        (graph, path)
        for graph, path in matches
        if _task_dag_graph_path_family(ctx, path) == preferred_family
    ]
    if len(preferred_group) > 1:
        paths = ", ".join(_task_dag_relative_path(ctx, path) for _, path in preferred_group)
        raise ValueError(
            f"Multiple task graph JSON artifacts match plan {plan_id!r} under {preferred_family}: {paths}. "
            f"Use --from-json <path>. {_TASK_DAG_CANONICAL_PATH_HINT}"
        )
    return preferred_graph, preferred_path
