from __future__ import annotations

from pathlib import Path
from typing import Any, Mapping

from .plan_graph import build_task_graph_progress, load_task_graph


TASK_DAG_PROGRESS_TRIGGERS = (
    "task dag",
    "task-dag",
    "dag",
    "task graph",
    "plan graph",
    "plan progress",
    "scheduler",
)

TASK_DAG_GRAPH_DOCS_SUBDIRS = ("sprints",)


def should_render_task_dag_progress(
    *,
    text: str | None,
    session: Mapping[str, Any] | None = None,
    surface_title: str | None = None,
) -> bool:
    haystack_parts = [text or "", surface_title or ""]
    if isinstance(session, Mapping):
        haystack_parts.append(str(session.get("title") or ""))
        metadata = session.get("metadata")
        if isinstance(metadata, Mapping):
            if metadata.get("task_graph_json") or metadata.get("task_graph_id"):
                return True
            haystack_parts.extend(str(metadata.get(key) or "") for key in ("plan_id", "plan_item_ref", "topic"))
    haystack = " ".join(part.lower() for part in haystack_parts if part)
    return any(trigger in haystack for trigger in TASK_DAG_PROGRESS_TRIGGERS)


def discover_task_dag_graph_paths(repo_root: Path) -> list[Path]:
    roots = [repo_root / "docs" / name for name in TASK_DAG_GRAPH_DOCS_SUBDIRS]
    candidates_by_path: dict[str, Path] = {}
    for docs_root in roots:
        if not docs_root.exists():
            continue
        for candidate in docs_root.rglob("*task_graph*.json"):
            candidates_by_path[str(candidate.resolve())] = candidate
    candidates = [candidates_by_path[key] for key in sorted(candidates_by_path)]
    return sorted(candidates, key=_task_dag_graph_sort_key)


def load_conversation_task_dag_graph(
    repo_root: Path,
    *,
    repo_name: str | None = None,
    task_graph_json: str | None = None,
    plan_id: str | None = None,
) -> tuple[dict[str, Any], Path] | None:
    explicit_path = _resolve_task_dag_graph_path(repo_root, task_graph_json)
    if explicit_path is not None:
        loaded = _load_matching_task_dag_graph(explicit_path, repo_name=repo_name, plan_id=plan_id)
        if loaded is not None:
            return loaded

    for path in discover_task_dag_graph_paths(repo_root):
        loaded = _load_matching_task_dag_graph(path, repo_name=repo_name, plan_id=plan_id)
        if loaded is not None:
            return loaded
    return None


def _resolve_task_dag_graph_path(repo_root: Path, task_graph_json: str | None) -> Path | None:
    value = str(task_graph_json or "").strip()
    if not value:
        return None
    path = Path(value)
    if not path.is_absolute():
        path = repo_root / path
    try:
        resolved = path.resolve()
        repo_resolved = repo_root.resolve()
    except OSError:
        return None
    try:
        resolved.relative_to(repo_resolved)
    except ValueError:
        return None
    return resolved if resolved.is_file() else None


def _load_matching_task_dag_graph(
    path: Path,
    *,
    repo_name: str | None = None,
    plan_id: str | None = None,
) -> tuple[dict[str, Any], Path] | None:
    try:
        graph = load_task_graph(path)
    except ValueError:
        return None
    graph_repo = str(graph.get("repo_name") or "").strip()
    if repo_name and graph_repo and graph_repo != repo_name:
        return None
    if plan_id and _task_dag_graph_plan_id(graph) != plan_id:
        return None
    return graph, path


def _task_dag_graph_plan_id(graph: Mapping[str, Any]) -> str | None:
    source_plan = graph.get("source_plan")
    if isinstance(source_plan, Mapping):
        value = str(source_plan.get("plan_id") or "").strip()
        if value:
            return value
    value = str(graph.get("plan_id") or "").strip()
    return value or None


def render_task_dag_conversation_progress(
    graph: dict[str, Any],
    readiness: Mapping[str, Any],
    *,
    max_blockers: int = 2,
) -> dict[str, Any]:
    progress = build_task_graph_progress(
        graph,
        _node_states_from_readiness(readiness),
        next_action=_readiness_next_action(readiness),
    )
    blockers = _conversation_blockers(readiness, max_blockers=max_blockers)
    lines = [_format_task_dag_progress_line(progress)]
    if blockers:
        lines.append("Blocked: " + "; ".join(f"{row['node_id']} - {row['reason']}" for row in blockers))
    return {
        "text": "\n".join(line for line in lines if line),
        "graph_id": graph.get("graph_id"),
        "source_plan": graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {},
        "progress": progress,
        "blockers": blockers,
    }


def _task_dag_graph_sort_key(path: Path) -> tuple[int, int, str]:
    text = path.as_posix()
    if "/docs/sprints/" in text or text.startswith("docs/sprints/"):
        root_rank = 0
    else:
        root_rank = len(TASK_DAG_GRAPH_DOCS_SUBDIRS)
    text = str(path)
    if "parallel_execution" in text:
        rank = 0
    elif "task_dag_scheduler" in text:
        rank = 1
    else:
        rank = 2
    return root_rank, rank, text


def _node_states_from_readiness(readiness: Mapping[str, Any]) -> dict[str, dict[str, Any]]:
    states: dict[str, dict[str, Any]] = {}
    raw_nodes = readiness.get("nodes")
    if not isinstance(raw_nodes, list):
        return states
    for row in raw_nodes:
        if not isinstance(row, Mapping):
            continue
        node_id = str(row.get("node_id") or "").strip()
        if not node_id:
            continue
        states[node_id] = {
            "state": str(row.get("state") or "blocked"),
            "reason": row.get("reason"),
        }
    return states


def _readiness_next_action(readiness: Mapping[str, Any]) -> str | None:
    summary = readiness.get("summary")
    if not isinstance(summary, Mapping):
        return None
    value = summary.get("next_action")
    return str(value).strip() if value else None


def _format_task_dag_progress_line(progress: Mapping[str, Any]) -> str:
    completed_percent = int(progress.get("completed_percent") or 0)
    estimated_percent = progress.get("estimated_percent")
    estimate_text = f" (~{int(estimated_percent)}% active)" if isinstance(estimated_percent, int) else ""
    return (
        f"DAG {completed_percent}% complete{estimate_text} · "
        f"done {int(progress.get('completed_nodes') or 0)}/{int(progress.get('total_nodes') or 0)} · "
        f"running {int(progress.get('running_nodes') or 0)} · "
        f"ready {int(progress.get('ready_nodes') or 0)} · "
        f"blocked {int(progress.get('blocked_nodes') or 0)} · "
        f"next: {str(progress.get('next_action') or 'none')}"
    )


def _conversation_blockers(readiness: Mapping[str, Any], *, max_blockers: int) -> list[dict[str, str]]:
    summary = readiness.get("summary")
    if isinstance(summary, Mapping) and int(summary.get("ready_nodes") or 0) > 0:
        return []
    rows: list[dict[str, str]] = []
    raw_nodes = readiness.get("nodes")
    if not isinstance(raw_nodes, list):
        return rows
    for row in raw_nodes:
        if not isinstance(row, Mapping) or row.get("state") != "blocked":
            continue
        node_id = str(row.get("node_id") or "").strip()
        reason = _compact_reason(str(row.get("reason") or "blocked"))
        if node_id and reason:
            rows.append({"node_id": node_id, "reason": reason})
        if max_blockers and len(rows) >= max_blockers:
            break
    return rows


def _compact_reason(value: str, limit: int = 140) -> str:
    text = " ".join(value.split()).strip()
    if len(text) <= limit:
        return text
    return text[: max(limit - 1, 0)].rstrip() + "…"
