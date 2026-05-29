from __future__ import annotations

import base64
import hashlib
import inspect
import shutil
import sys
from pathlib import Path
from typing import Any

from ..local_content_bundle import export_snapshot_bundle as _fallback_export_snapshot_bundle
from ..plan_graph import topological_node_order
from ..remote_client import (
    RemoteError,
    create_change as _fallback_remote_create_change,
    get_change as _fallback_remote_get_change,
    create_session as _fallback_remote_create_session,
    create_task as _fallback_remote_create_task,
)
from ..store_local_changes import get_local_change as _fallback_get_local_change
from ..snapshot_diff import diff_snapshot_file_maps as _fallback_diff_snapshot_file_maps
from ..store import (
    RepoContext,
    add_worktree as _fallback_local_add_worktree,
    bind_worktree as _fallback_local_bind_worktree,
    collect_snapshot_chain as _fallback_collect_snapshot_chain,
    create_local_change as _fallback_create_local_change,
    create_local_task as _fallback_create_local_task,
    current_line as _fallback_current_line,
    get_line as _fallback_get_line,
    load_config as _fallback_load_config,
    workspace_status as _fallback_workspace_status,
)
from ..task_dag_readiness import build_task_dag_promotion_policy as _fallback_build_task_dag_promotion_policy
from .remote_session_wrappers import (
    remote_append_session_event as _fallback_remote_append_session_event,
    remote_get_session as _fallback_remote_get_session,
)
from .runtime_defaults import (
    _effective_author_mode as _fallback_effective_author_mode,
    _effective_model_name as _fallback_effective_model_name,
    _normalize_text_value as _fallback_normalize_text_value,
)
from .task_dag_compact_packet_authoring import (
    _task_dag_compact_packet_surface_payload as _fallback_task_dag_compact_packet_surface_payload,
)
from .task_dag_readiness_views import (
    _task_dag_change_focus_policy as _fallback_task_dag_change_focus_policy,
    _task_dag_node_index as _fallback_task_dag_node_index,
    _task_dag_view_row_by_node_id as _fallback_task_dag_view_row_by_node_id,
)
from .task_dag_runtime_helpers import (
    _task_dag_relative_path as _fallback_task_dag_relative_path,
    _task_dag_remote_plan_revision_id as _fallback_task_dag_remote_plan_revision_id,
    _task_dag_target_line_name as _fallback_task_dag_target_line_name,
)
from .task_dag_topology_helpers import (
    _task_dag_converged_output_node_ids as _fallback_task_dag_converged_output_node_ids,
    _task_dag_node_workflow_boundary as _fallback_task_dag_node_workflow_boundary,
)
from .task_tracking_bindings import _task_worktree_repo_ctx as _fallback_task_worktree_repo_ctx
from .task_worktree_guidance import _task_worktree_output as _fallback_task_worktree_output
from .task_worktree_resolution import (
    _change_bootstrap_lineage as _fallback_change_bootstrap_lineage,
    _ensure_task_feature_line as _fallback_ensure_task_feature_line,
    _find_bound_task_worktree as _fallback_find_bound_task_worktree,
    _resolve_task_bound_worktree_name as _fallback_resolve_task_bound_worktree_name,
    _task_feature_line_name as _fallback_task_feature_line_name,
)
from .workflow_authoring import _remote_change_lineage_payload as _fallback_remote_change_lineage_payload
from .workflow_mode_config import _effective_workflow_mode as _fallback_effective_workflow_mode


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is None:
        return fallback
    return getattr(app_module, name, fallback)


def remote_create_task(*args: Any, **kwargs: Any) -> Any:
    return _app_override("remote_create_task", _fallback_remote_create_task)(*args, **kwargs)


def create_local_task(*args: Any, **kwargs: Any) -> Any:
    return _app_override("create_local_task", _fallback_create_local_task)(*args, **kwargs)


def create_local_change(*args: Any, **kwargs: Any) -> Any:
    return _app_override("create_local_change", _fallback_create_local_change)(*args, **kwargs)


def remote_create_change(*args: Any, **kwargs: Any) -> Any:
    return _app_override("remote_create_change", _fallback_remote_create_change)(*args, **kwargs)


def remote_get_change(*args: Any, **kwargs: Any) -> Any:
    return _app_override("remote_get_change", _fallback_remote_get_change)(*args, **kwargs)


def get_local_change(*args: Any, **kwargs: Any) -> Any:
    return _app_override("get_local_change", _fallback_get_local_change)(*args, **kwargs)


def remote_create_session(*args: Any, **kwargs: Any) -> Any:
    return _app_override("remote_create_session", _fallback_remote_create_session)(*args, **kwargs)


def remote_get_session(*args: Any, **kwargs: Any) -> Any:
    return _app_override("remote_get_session", _fallback_remote_get_session)(*args, **kwargs)


def remote_append_session_event(*args: Any, **kwargs: Any) -> Any:
    return _app_override("remote_append_session_event", _fallback_remote_append_session_event)(*args, **kwargs)


def load_config(ctx: RepoContext) -> dict[str, Any]:
    return _app_override("load_config", _fallback_load_config)(ctx)


def workspace_status(ctx: RepoContext, *, snapshot_id: str | None = None, line_name: str | None = None) -> dict[str, Any]:
    return _app_override("workspace_status", _fallback_workspace_status)(
        ctx,
        snapshot_id=snapshot_id,
        line_name=line_name,
    )


def current_line(ctx: RepoContext) -> str:
    return _app_override("current_line", _fallback_current_line)(ctx)


def get_line(ctx: RepoContext, name: str) -> dict[str, Any]:
    return _app_override("get_line", _fallback_get_line)(ctx, name)


def collect_snapshot_chain(ctx: RepoContext, snapshot_id: str) -> list[str]:
    return _app_override("collect_snapshot_chain", _fallback_collect_snapshot_chain)(ctx, snapshot_id)


def export_snapshot_bundle(ctx: RepoContext, snapshot_id: str, repo_name: str) -> dict[str, Any]:
    exporter = _app_override("export_snapshot_bundle", _fallback_export_snapshot_bundle)
    try:
        parameters = tuple(inspect.signature(exporter).parameters.values())
    except (TypeError, ValueError):
        parameters = ()
    positional_parameters = [
        parameter
        for parameter in parameters
        if parameter.kind in (inspect.Parameter.POSITIONAL_ONLY, inspect.Parameter.POSITIONAL_OR_KEYWORD)
    ]
    accepts_varargs = any(parameter.kind is inspect.Parameter.VAR_POSITIONAL for parameter in parameters)
    if accepts_varargs or len(positional_parameters) >= 3:
        return exporter(ctx, snapshot_id, repo_name)
    return exporter(ctx, snapshot_id)


def diff_snapshot_file_maps(*args: Any, **kwargs: Any) -> dict[str, Any]:
    return _app_override("diff_snapshot_file_maps", _fallback_diff_snapshot_file_maps)(*args, **kwargs)


def local_add_worktree(*args: Any, **kwargs: Any) -> Any:
    return _app_override("local_add_worktree", _fallback_local_add_worktree)(*args, **kwargs)


def local_bind_worktree(*args: Any, **kwargs: Any) -> Any:
    return _app_override("local_bind_worktree", _fallback_local_bind_worktree)(*args, **kwargs)


def _effective_author_mode(ctx: RepoContext) -> str:
    return _app_override("_effective_author_mode", _fallback_effective_author_mode)(ctx)


def _effective_model_name(ctx: RepoContext) -> str | None:
    return _app_override("_effective_model_name", _fallback_effective_model_name)(ctx)


def _normalize_text_value(value: str | None) -> str | None:
    return _app_override("_normalize_text_value", _fallback_normalize_text_value)(value)


def _task_dag_relative_path(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_relative_path", _fallback_task_dag_relative_path)(*args, **kwargs)


def _task_dag_target_line_name(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_target_line_name", _fallback_task_dag_target_line_name)(*args, **kwargs)


def _task_dag_remote_plan_revision_id(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_remote_plan_revision_id", _fallback_task_dag_remote_plan_revision_id)(*args, **kwargs)


def _task_dag_view_row_by_node_id(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_view_row_by_node_id", _fallback_task_dag_view_row_by_node_id)(*args, **kwargs)


def _task_dag_change_focus_policy(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_change_focus_policy", _fallback_task_dag_change_focus_policy)(*args, **kwargs)


def _task_dag_node_index(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_node_index", _fallback_task_dag_node_index)(*args, **kwargs)


def _task_dag_node_workflow_boundary(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_node_workflow_boundary", _fallback_task_dag_node_workflow_boundary)(*args, **kwargs)


def _task_dag_converged_output_node_ids(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_converged_output_node_ids", _fallback_task_dag_converged_output_node_ids)(*args, **kwargs)


def _task_dag_compact_packet_surface_payload(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_compact_packet_surface_payload", _fallback_task_dag_compact_packet_surface_payload)(*args, **kwargs)


def _effective_workflow_mode(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_effective_workflow_mode", _fallback_effective_workflow_mode)(*args, **kwargs)


def _remote_change_lineage_payload(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_remote_change_lineage_payload", _fallback_remote_change_lineage_payload)(*args, **kwargs)


def _task_worktree_repo_ctx(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_worktree_repo_ctx", _fallback_task_worktree_repo_ctx)(*args, **kwargs)


def _task_worktree_output(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_worktree_output", _fallback_task_worktree_output)(*args, **kwargs)


def _ensure_task_feature_line(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_ensure_task_feature_line", _fallback_ensure_task_feature_line)(*args, **kwargs)


def _find_bound_task_worktree(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_find_bound_task_worktree", _fallback_find_bound_task_worktree)(*args, **kwargs)


def _resolve_task_bound_worktree_name(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_resolve_task_bound_worktree_name", _fallback_resolve_task_bound_worktree_name)(*args, **kwargs)


def _task_feature_line_name(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_feature_line_name", _fallback_task_feature_line_name)(*args, **kwargs)


def _change_bootstrap_lineage(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_change_bootstrap_lineage", _fallback_change_bootstrap_lineage)(*args, **kwargs)


def build_task_dag_promotion_policy(*args: Any, **kwargs: Any) -> Any:
    return _app_override("build_task_dag_promotion_policy", _fallback_build_task_dag_promotion_policy)(*args, **kwargs)


def _task_dag_final_remote_disposition_default(*args: Any, **kwargs: Any) -> Any:
    fn = _app_override("_task_dag_final_remote_disposition_default", None)
    if fn is None:
        raise RuntimeError(
            "ait.cli.app did not expose the final remote disposition helper required for task-DAG node bootstrap."
        )
    return fn(*args, **kwargs)


def _task_dag_completed_ancestor_node_ids(graph: dict[str, Any], readiness: dict[str, Any], node_id: str) -> list[str]:
    graph_nodes = _task_dag_node_index(graph)
    if not graph_nodes:
        return []
    reverse_dependencies: dict[str, set[str]] = {candidate: set() for candidate in graph_nodes}
    for candidate_id, candidate_node in graph_nodes.items():
        if not isinstance(candidate_node, dict):
            continue
        for parent in candidate_node.get("depends_on") or []:
            parent_id = str(parent or "").strip()
            if parent_id and parent_id in reverse_dependencies:
                reverse_dependencies.setdefault(candidate_id, set()).add(parent_id)
    pending = [str(node_id)]
    ancestors: set[str] = set()
    while pending:
        current = pending.pop()
        for parent_id in reverse_dependencies.get(current, set()):
            if parent_id in ancestors:
                continue
            ancestors.add(parent_id)
            pending.append(parent_id)
    readiness_by_node = {
        str(row.get("node_id") or "").strip(): row
        for row in readiness.get("nodes") or []
        if isinstance(row, dict) and str(row.get("node_id") or "").strip()
    }
    return [
        candidate
        for candidate in topological_node_order(graph)
        if candidate in ancestors
        and str((readiness_by_node.get(candidate) or {}).get("state") or "").strip().lower() == "completed"
    ]


def _task_dag_readiness_row_task_id(row: dict[str, Any]) -> str | None:
    lineage = row.get("lineage") if isinstance(row.get("lineage"), dict) else {}
    return _normalize_text_value(row.get("task_id")) or _normalize_text_value(lineage.get("task_id"))


def _task_dag_worktree_root(worktree: dict[str, Any]) -> Path:
    raw = (
        _normalize_text_value(worktree.get("path"))
        or _normalize_text_value(worktree.get("workspace_root"))
        or _normalize_text_value(worktree.get("open_path"))
        or _normalize_text_value(worktree.get("alias_path"))
    )
    if raw is None:
        raise ValueError("Task DAG worktree payload is missing a usable workspace path.")
    return Path(raw).expanduser().resolve()


def _task_dag_file_fingerprint(path: Path) -> tuple[str, int]:
    return hashlib.sha256(path.read_bytes()).hexdigest(), path.stat().st_mode & 0o777


def _task_dag_readiness_row_lineage(row: dict[str, Any]) -> dict[str, Any]:
    return row.get("lineage") if isinstance(row.get("lineage"), dict) else {}


def _task_dag_snapshot_bundle_file_map(bundle: dict[str, Any]) -> dict[str, dict[str, Any]]:
    files = bundle.get("files") if isinstance(bundle.get("files"), list) else []
    return {
        str(entry.get("path") or ""): dict(entry)
        for entry in files
        if isinstance(entry, dict) and str(entry.get("path") or "").strip()
    }


def _task_dag_snapshot_bundle_mode(value: Any) -> int:
    text = str(value or "0o644").strip() or "0o644"
    try:
        return int(text, 0)
    except ValueError:
        return int(text, 8)


def _task_dag_dependency_replay_snapshot_ids(
    dependency_row: dict[str, Any],
    *,
    dependency_node_id: str,
    node_id: str,
) -> tuple[str, str]:
    lineage = _task_dag_readiness_row_lineage(dependency_row)
    completion_snapshot_id = _normalize_text_value(lineage.get("completion_snapshot_id"))
    landed_snapshot_id = _normalize_text_value(lineage.get("landed_snapshot_id"))
    completion_fork_snapshot_id = _normalize_text_value(lineage.get("completion_fork_snapshot_id"))
    replay_snapshot_id = landed_snapshot_id or completion_snapshot_id
    if replay_snapshot_id is None:
        raise ValueError(
            f"Completed dependency node `{dependency_node_id}` is missing frozen snapshot evidence required to converge `{node_id}`."
        )
    if completion_fork_snapshot_id is None:
        raise ValueError(
            f"Completed dependency node `{dependency_node_id}` is missing completion_fork_snapshot_id required to converge `{node_id}`."
        )
    return completion_fork_snapshot_id, replay_snapshot_id


def _task_dag_dependency_replay_guard_message(
    *,
    dependency_row: dict[str, Any],
    dependency_node_id: str,
    node_id: str,
    repo_ctx: RepoContext,
) -> str | None:
    lineage = _task_dag_readiness_row_lineage(dependency_row)
    if _normalize_text_value(lineage.get("landed_snapshot_id")) is not None:
        return None
    dependency_task_id = _task_dag_readiness_row_task_id(dependency_row)
    if dependency_task_id is None:
        return None
    dependency_worktree = _find_bound_task_worktree(repo_ctx, dependency_task_id)
    if not isinstance(dependency_worktree, dict):
        return None
    retarget = dependency_worktree.get("retarget") if isinstance(dependency_worktree.get("retarget"), dict) else {}
    worktree_name = _normalize_text_value(dependency_worktree.get("name")) or dependency_task_id
    target_base_line = (
        _normalize_text_value(retarget.get("target_base_line"))
        or _normalize_text_value(dependency_worktree.get("target_base_line"))
        or "main"
    )
    replay_snapshot_id = _task_dag_dependency_replay_target_snapshot_id(dependency_row)
    target_base_snapshot_id = (
        _normalize_text_value(retarget.get("target_base_snapshot_id"))
        or _normalize_text_value(dependency_worktree.get("target_base_snapshot_id"))
        or _task_dag_line_head_snapshot_id(repo_ctx, target_base_line)
    )
    if _task_dag_snapshot_is_on_target_line(
        repo_ctx,
        replay_snapshot_id,
        target_base_snapshot_id,
    ):
        return None
    rebase_state = str(retarget.get("rebase_state") or dependency_worktree.get("rebase_state") or "idle")
    if rebase_state == "conflicted":
        conflict_paths = retarget.get("rebase_conflict_paths") or dependency_worktree.get("rebase_conflict_paths") or []
        sample = ", ".join(str(path).strip() for path in conflict_paths[:5] if str(path).strip()) or "resolve conflicts first"
        return (
            f"Converged node `{node_id}` cannot replay dependency node `{dependency_node_id}` because "
            f"bound worktree `{worktree_name}` is paused on a conflicted rebase: {sample}. "
            f"Run `ait worktree rebase --continue` or `ait worktree rebase --abort` in `{worktree_name}` first."
        )
    if bool(retarget.get("needs_retarget") or dependency_worktree.get("needs_retarget")):
        fork_snapshot_id = (
            _normalize_text_value(retarget.get("fork_snapshot_id"))
            or _normalize_text_value(dependency_worktree.get("fork_snapshot_id"))
            or "unknown"
        )
        return (
            f"Converged node `{node_id}` cannot replay dependency node `{dependency_node_id}` because "
            f"bound worktree `{worktree_name}` still forks from `{fork_snapshot_id}` while "
            f"`{target_base_line}` now points at `{target_base_snapshot_id or 'unknown'}`. "
            f"Run `ait worktree rebase --onto {target_base_line}` in `{worktree_name}` first."
        )
    return None


def _task_dag_dependency_replay_target_snapshot_id(dependency_row: dict[str, Any]) -> str | None:
    lineage = _task_dag_readiness_row_lineage(dependency_row)
    return _normalize_text_value(lineage.get("landed_snapshot_id")) or _normalize_text_value(
        lineage.get("completion_snapshot_id")
    )


def _task_dag_line_head_snapshot_id(repo_ctx: RepoContext, line_name: str) -> str | None:
    try:
        line = get_line(repo_ctx, line_name)
    except (KeyError, ValueError):
        return None
    return _normalize_text_value(line.get("head_snapshot_id"))


def _task_dag_snapshot_is_on_target_line(
    repo_ctx: RepoContext,
    snapshot_id: str | None,
    target_head_snapshot_id: str | None,
) -> bool:
    if snapshot_id is None or target_head_snapshot_id is None:
        return False
    if snapshot_id == target_head_snapshot_id:
        return True
    try:
        return snapshot_id in collect_snapshot_chain(repo_ctx, target_head_snapshot_id)
    except (KeyError, ValueError):
        return False


def _task_dag_replay_dependency_outputs_onto_worktree(
    *,
    ctx: RepoContext,
    graph: dict[str, Any],
    readiness: dict[str, Any],
    node_id: str,
    worktree: dict[str, Any],
) -> dict[str, Any]:
    dependency_node_ids = _task_dag_completed_ancestor_node_ids(graph, readiness, node_id)
    if not dependency_node_ids:
        return {"replayed_node_ids": [], "replayed_paths": [], "deleted_paths": []}
    repo_ctx = _task_worktree_repo_ctx(ctx)
    readiness_by_node = {
        str(row.get("node_id") or "").strip(): row
        for row in readiness.get("nodes") or []
        if isinstance(row, dict) and str(row.get("node_id") or "").strip()
    }
    target_root = _task_dag_worktree_root(worktree)
    applied: dict[str, dict[str, Any]] = {}
    replayed_node_ids: list[str] = []
    replayed_paths: set[str] = set()
    deleted_paths: set[str] = set()
    pending_upserts: dict[str, dict[str, Any]] = {}
    pending_deletes: set[str] = set()
    for dependency_node_id in dependency_node_ids:
        dependency_row = readiness_by_node.get(dependency_node_id)
        if dependency_row is None:
            continue
        guard_message = _task_dag_dependency_replay_guard_message(
            dependency_row=dependency_row,
            dependency_node_id=dependency_node_id,
            node_id=node_id,
            repo_ctx=repo_ctx,
        )
        if guard_message is not None:
            raise ValueError(guard_message)
        baseline_snapshot_id, replay_snapshot_id = _task_dag_dependency_replay_snapshot_ids(
            dependency_row,
            dependency_node_id=dependency_node_id,
            node_id=node_id,
        )
        repo_name = str(graph.get("repo_name") or "").strip()
        baseline_bundle = export_snapshot_bundle(repo_ctx, baseline_snapshot_id, repo_name)
        replay_bundle = export_snapshot_bundle(repo_ctx, replay_snapshot_id, repo_name)
        baseline_files = _task_dag_snapshot_bundle_file_map(baseline_bundle)
        replay_files = _task_dag_snapshot_bundle_file_map(replay_bundle)
        delta = diff_snapshot_file_maps(
            baseline_files,
            replay_files,
            old_snapshot_id=baseline_snapshot_id,
            new_snapshot_id=replay_snapshot_id,
        )
        changed = False
        for relative_path in sorted(
            {
                *[str(path) for path in delta.get("added") or []],
                *[str(path) for path in delta.get("modified") or []],
                *[str(path) for path in delta.get("mode_changed") or []],
            }
        ):
            source_row = replay_files.get(relative_path)
            if not isinstance(source_row, dict):
                raise ValueError(
                    f"Completed dependency node `{dependency_node_id}` is missing frozen replay source `{relative_path}`."
                )
            encoded = _normalize_text_value(source_row.get("content_b64"))
            if encoded is None:
                raise ValueError(
                    f"Completed dependency node `{dependency_node_id}` is missing snapshot content required to replay `{relative_path}`."
                )
            source_bytes = base64.b64decode(encoded)
            file_hash = _normalize_text_value(source_row.get("sha256")) or hashlib.sha256(source_bytes).hexdigest()
            file_mode = _task_dag_snapshot_bundle_mode(source_row.get("mode"))
            marker = ("upsert", file_hash, file_mode)
            existing = applied.get(relative_path)
            if existing is not None and existing.get("marker") != marker:
                raise ValueError(
                    f"Converged node `{node_id}` cannot replay conflicting dependency outputs before worker handoff: "
                    f"`{relative_path}` differs between dependency nodes `{existing['dependency_node_id']}` and `{dependency_node_id}`."
                )
            applied[relative_path] = {
                "marker": marker,
                "dependency_node_id": dependency_node_id,
            }
            pending_upserts[relative_path] = {
                "source_bytes": source_bytes,
                "file_mode": file_mode,
            }
            replayed_paths.add(relative_path)
            changed = True
        for relative_path in sorted(str(path) for path in delta.get("deleted") or []):
            marker = ("delete", None, None)
            existing = applied.get(relative_path)
            if existing is not None and existing.get("marker") != marker:
                raise ValueError(
                    f"Converged node `{node_id}` cannot replay conflicting dependency outputs before worker handoff: "
                    f"`{relative_path}` differs between dependency nodes `{existing['dependency_node_id']}` and `{dependency_node_id}`."
                )
            applied[relative_path] = {
                "marker": marker,
                "dependency_node_id": dependency_node_id,
            }
            pending_deletes.add(relative_path)
            deleted_paths.add(relative_path)
            changed = True
        if changed:
            replayed_node_ids.append(dependency_node_id)
    for relative_path in sorted(pending_upserts):
        target_path = target_root / relative_path
        payload = pending_upserts[relative_path]
        target_path.parent.mkdir(parents=True, exist_ok=True)
        target_path.write_bytes(payload["source_bytes"])
        target_path.chmod(payload["file_mode"])
    for relative_path in sorted(pending_deletes):
        target_path = target_root / relative_path
        if target_path.exists():
            target_path.unlink()
            parent = target_path.parent
            while parent != target_root and parent.exists():
                try:
                    parent.rmdir()
                except OSError:
                    break
                parent = parent.parent
    return {
        "replayed_node_ids": replayed_node_ids,
        "replayed_paths": sorted(replayed_paths),
        "deleted_paths": sorted(deleted_paths),
    }


def _task_dag_create_task_for_node(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    plan_revision_id: str | None,
    graph: dict[str, Any],
    graph_node: dict[str, Any],
) -> dict[str, Any]:
    template = graph_node.get("task_template") if isinstance(graph_node.get("task_template"), dict) else {}
    node_id = str(graph_node.get("node_id") or "")
    title = str(template.get("title") or graph_node.get("title") or f"Task DAG node {node_id}")
    risk_tier = str(template.get("risk_tier") or "medium")
    plan_item_ref = str(graph_node.get("plan_item_ref") or "")
    intent = str(
        template.get("intent")
        or f"Execute Task DAG node {node_id} for plan item {plan_item_ref or 'unbound'}."
    )
    workflow_boundary = _task_dag_node_workflow_boundary(graph_node)
    final_remote_disposition_default = _task_dag_final_remote_disposition_default(ctx, graph)
    if workflow_boundary == "execution_only" or not final_remote_disposition_default:
        return create_local_task(
            ctx,
            title,
            intent,
            risk_tier,
            plan_id=plan_id,
            origin_plan_revision_id=plan_revision_id,
            plan_item_ref=plan_item_ref or None,
        )
    remote_name = _normalize_text_value(remote_row.get("name")) or _normalize_text_value(load_config(ctx).get("default_remote"))
    remote_plan_revision_id = _task_dag_remote_plan_revision_id(
        ctx,
        graph,
        remote_name=remote_name,
        auto_publish_if_needed=True,
    )
    return remote_create_task(
        remote_row["url"],
        repo_name,
        title,
        intent,
        risk_tier,
        plan_id=plan_id,
        origin_plan_revision_id=remote_plan_revision_id or plan_revision_id,
        plan_item_ref=plan_item_ref or None,
    )


def _task_dag_create_batch_session(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    plan_revision_id: str | None,
    graph: dict[str, Any],
    graph_path: Path,
    readiness: dict[str, Any],
    execution_strategy: dict[str, Any],
    batch: dict[str, Any],
    created_task_by_node: dict[str, dict[str, Any]],
) -> dict[str, Any]:
    graph_id = str(graph.get("graph_id") or "")
    batch_id = str(batch.get("batch_id") or "batch")
    node_ids = [str(node_id) for node_id in batch.get("node_ids") or [] if str(node_id)]
    graph_artifact_path = _task_dag_relative_path(ctx, graph_path)
    node_task_ids = {
        node_id: str(task.get("task_id"))
        for node_id, task in created_task_by_node.items()
        if node_id in node_ids and task.get("task_id")
    }
    batch_rows: list[dict[str, Any]] = []
    for node_id in node_ids:
        row = _task_dag_view_row_by_node_id(readiness, node_id) or {
            "node_id": node_id,
            "state": "ready",
            "workflow_state": "ready",
        }
        normalized = dict(row)
        if node_task_ids.get(node_id) and not normalized.get("task_id"):
            normalized["task_id"] = node_task_ids[node_id]
        batch_rows.append(normalized)
    change_focus_policy = _task_dag_change_focus_policy(
        batch_rows,
        execution_only_node_ids=execution_strategy.get("execution_only_node_ids") or [],
    )
    metadata: dict[str, Any] = {
        "author_mode": _effective_author_mode(ctx),
        "session_policy": "task_dag_compact_packet_worker",
        "dispatch_model": execution_strategy.get("dispatch_model") or "compact_packet",
        "worker_execution_mode": execution_strategy.get("worker_execution_mode"),
        "worker_execution_label": execution_strategy.get("worker_execution_label"),
        "fresh_worker_session": execution_strategy.get("fresh_worker_session"),
        "worker_session_count": execution_strategy.get("worker_session_count"),
        "worker_session_mode": execution_strategy.get("worker_session_mode") or "single_fresh_worker_session",
        "max_total_sessions": execution_strategy.get("max_total_sessions"),
        "max_worker_sessions": execution_strategy.get("max_worker_sessions"),
        "physical_fanout": bool(execution_strategy.get("physical_fanout_default")),
        "plan_id": plan_id,
        "graph_id": graph_id,
        "task_graph_json": graph_artifact_path,
        "batch_id": batch_id,
        "node_ids": node_ids,
        "change_focus_policy": change_focus_policy,
        "next_focus_node_id": (
            (change_focus_policy.get("next_focus") or {}).get("node_id")
            if isinstance(change_focus_policy.get("next_focus"), dict)
            else None
        ),
        "next_focus_task_id": (
            (change_focus_policy.get("next_focus") or {}).get("task_id")
            if isinstance(change_focus_policy.get("next_focus"), dict)
            else None
        ),
        "next_focus_change_id": (
            (change_focus_policy.get("next_focus") or {}).get("change_id")
            if isinstance(change_focus_policy.get("next_focus"), dict)
            else None
        ),
    }
    if plan_revision_id:
        metadata["plan_revision_id"] = plan_revision_id
    if node_task_ids:
        metadata["node_task_ids"] = node_task_ids
    metadata["compact_packet_surface"] = _task_dag_compact_packet_surface_payload(
        ctx,
        graph_path=graph_path,
        change_focus_policy=change_focus_policy,
    )
    session = remote_create_session(
        remote_row["url"],
        repo_name,
        "agent_run",
        title=f"Task DAG compact worker {batch_id}: {graph_id}",
        line_name=current_line(ctx),
        worktree_name=_normalize_text_value(load_config(ctx).get("worktree_name")),
        model_name=_effective_model_name(ctx),
        metadata=metadata,
    )
    return {
        "batch_id": batch_id,
        "session_id": session.get("session_id"),
        "title": session.get("title"),
        "node_ids": node_ids,
        "node_task_ids": node_task_ids,
        "session_kind": session.get("session_kind") or session.get("kind") or "agent_run",
        "metadata": metadata,
    }


def _task_dag_materialize_node_lineage(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    graph: dict[str, Any],
    readiness: dict[str, Any],
    node_id: str,
    create_worktree: bool,
    allow_execution_only_without_change: bool = False,
) -> dict[str, Any]:
    graph_nodes = _task_dag_node_index(graph)
    graph_node = graph_nodes.get(str(node_id))
    if not isinstance(graph_node, dict):
        raise ValueError(f"Unknown node id: {node_id}")
    if str(graph_node.get("node_kind") or "") != "task":
        raise ValueError(f"Node {node_id} is not a task node.")
    target_line = _task_dag_target_line_name(ctx, graph, graph_node)
    row = _task_dag_view_row_by_node_id(readiness, str(node_id))
    if row is None:
        raise ValueError(f"No readiness evidence found for node {node_id}.")

    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    plan_revision_id = str(source_plan.get("plan_revision_id") or "").strip() or None
    template = graph_node.get("task_template") if isinstance(graph_node.get("task_template"), dict) else {}
    task_id = str(row.get("task_id") or "").strip()
    if not task_id:
        task = _task_dag_create_task_for_node(
            ctx=ctx,
            remote_row=remote_row,
            repo_name=repo_name,
            plan_id=plan_id,
            plan_revision_id=plan_revision_id,
            graph_node=graph_node,
            graph=graph,
        )
        task_id = str(task.get("task_id") or "")
    change_id = str(row.get("change_id") or "").strip()
    created_change = None
    converged_output_node_ids = {
        str(candidate)
        for candidate in (_task_dag_converged_output_node_ids(graph) or [])
        if str(candidate)
    }
    shared_boundary_node = str(node_id) in converged_output_node_ids
    final_remote_disposition_default = _task_dag_final_remote_disposition_default(ctx, graph)
    workflow_boundary = "reviewable_output" if shared_boundary_node else "execution_only"
    local_first_execution_only = not shared_boundary_node or not final_remote_disposition_default
    if change_id and not shared_boundary_node:
        raise ValueError(
            f"Node {node_id} is not the final converged DAG node and cannot carry a shared / remote change under the single-path DAG contract."
        )
    if not change_id and shared_boundary_node:
        change_title = str(template.get("change_title") or graph_node.get("title") or f"Task DAG node {node_id}")
        risk_tier = str(template.get("risk_tier") or "medium")
        if final_remote_disposition_default:
            created_change = remote_create_change(
                remote_row["url"],
                repo_name,
                task_id,
                change_title,
                base_line=target_line,
                risk_tier=risk_tier,
                **_remote_change_lineage_payload(remote_row["url"], repo_name, target_line),
            )
        else:
            created_change = create_local_change(
                ctx,
                task_id,
                change_title,
                target_line,
                risk_tier,
            )
        change_id = str(created_change.get("change_id") or "")

    resolved_change = dict(created_change) if isinstance(created_change, dict) else None
    resolved_change_id = str(change_id or "").strip() or None
    if resolved_change is None and resolved_change_id is not None:
        if final_remote_disposition_default and shared_boundary_node:
            try:
                resolved_change = remote_get_change(remote_row["url"], resolved_change_id, repo_name=repo_name)
            except (KeyError, RemoteError, ValueError):
                resolved_change = None
        else:
            try:
                resolved_change = get_local_change(ctx, resolved_change_id)
            except KeyError:
                resolved_change = None
        if resolved_change is None:
            raise ValueError(
                f"Cannot resolve change `{resolved_change_id}` lineage for DAG task-worktree bootstrap."
            )
    resolved_base_line_name, resolved_fork_snapshot_id = _change_bootstrap_lineage(
        resolved_change,
        fallback_base_line_name=target_line,
    )

    repo_ctx = _task_worktree_repo_ctx(ctx)
    feature_line = _ensure_task_feature_line(
        repo_ctx,
        task_id=task_id,
        base_line_name=resolved_base_line_name,
        base_snapshot_id=resolved_fork_snapshot_id,
    )
    feature_line_name = str(feature_line.get("line_name") or _task_feature_line_name(task_id))

    worktree = _find_bound_task_worktree(repo_ctx, task_id)
    created_worktree = None
    if create_worktree and worktree is None:
        worktree_name = _resolve_task_bound_worktree_name(
            repo_ctx,
            task_id,
            str(graph_node.get("title") or template.get("title") or task_id),
        )
        base_line = get_line(repo_ctx, resolved_base_line_name)
        bootstrap_fork_snapshot_id = resolved_fork_snapshot_id or _normalize_text_value(base_line.get("head_snapshot_id"))
        created_worktree = local_bind_worktree(
            repo_ctx,
            local_add_worktree(
                repo_ctx,
                worktree_name,
                line_name=feature_line_name,
                creation_kind="task_auto_created",
                cleanup_policy="after_remote_land",
            )["name"],
            task_id=task_id,
            change_id=resolved_change_id,
            auto_created_for_task=True,
            fork_snapshot_id=bootstrap_fork_snapshot_id,
            forked_from_line=resolved_base_line_name,
            target_base_line=resolved_base_line_name,
        )
        worktree = created_worktree
    if worktree is not None and (
        (resolved_change_id and str(worktree.get("bound_change_id") or "").strip() != resolved_change_id)
        or _normalize_text_value(worktree.get("target_base_line")) != resolved_base_line_name
    ):
        worktree = local_bind_worktree(
            repo_ctx,
            str(worktree.get("name") or ""),
            task_id=task_id,
            change_id=resolved_change_id,
            auto_created_for_task=bool(worktree.get("auto_created_for_task")),
            fork_snapshot_id=resolved_fork_snapshot_id,
            forked_from_line=resolved_base_line_name,
            target_base_line=resolved_base_line_name,
        )
    if not allow_execution_only_without_change and not shared_boundary_node and not change_id:
        raise ValueError(
            f"Node {node_id} requires a shared / remote change before continuing because execution-only bootstrap without change is disabled."
        )

    worktree_payload = _task_worktree_output(worktree) if isinstance(worktree, dict) else None
    dependency_replay = (
        _task_dag_replay_dependency_outputs_onto_worktree(
            ctx=ctx,
            graph=graph,
            readiness=readiness,
            node_id=str(node_id),
            worktree=worktree,
        )
        if shared_boundary_node and isinstance(worktree, dict)
        else {"replayed_node_ids": [], "replayed_paths": [], "deleted_paths": []}
    )
    session_line_name = (
        _normalize_text_value((worktree_payload or {}).get("current_line"))
        or _normalize_text_value((worktree_payload or {}).get("registered_line_name"))
        or feature_line_name
    )
    return {
        "graph_node": graph_node,
        "plan_revision_id": plan_revision_id,
        "task_id": task_id,
        "change_id": change_id or None,
        "created_change": created_change,
        "shared_boundary_node": shared_boundary_node,
        "workflow_boundary": workflow_boundary,
        "local_first_execution_only": local_first_execution_only,
        "worktree": worktree_payload,
        "created_worktree": _task_worktree_output(created_worktree) if isinstance(created_worktree, dict) else None,
        "dependency_replay": dependency_replay,
        "session_line_name": session_line_name,
    }


def _task_dag_bootstrap_node(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    graph: dict[str, Any],
    graph_path: Path,
    readiness: dict[str, Any],
    node_id: str,
    graph_run_session_id: str | None,
    create_worktree: bool,
    allow_execution_only_without_change: bool = False,
) -> dict[str, Any]:
    materialized = _task_dag_materialize_node_lineage(
        ctx=ctx,
        remote_row=remote_row,
        repo_name=repo_name,
        plan_id=plan_id,
        graph=graph,
        readiness=readiness,
        node_id=node_id,
        create_worktree=create_worktree,
        allow_execution_only_without_change=allow_execution_only_without_change,
    )
    graph_node = materialized["graph_node"]
    plan_revision_id = materialized["plan_revision_id"]
    task_id = str(materialized["task_id"])
    change_id = str(materialized.get("change_id") or "")
    created_change = materialized.get("created_change")
    shared_boundary_node = bool(materialized["shared_boundary_node"])
    workflow_boundary = str(materialized["workflow_boundary"])
    local_first_execution_only = bool(materialized["local_first_execution_only"])
    worktree_payload = materialized.get("worktree") if isinstance(materialized.get("worktree"), dict) else None
    session_line_name = str(materialized["session_line_name"])
    session_metadata: dict[str, Any] = {
        "author_mode": _effective_author_mode(ctx),
        "session_policy": "task_dag_node_bootstrap",
        "plan_id": plan_id,
        "plan_revision_id": plan_revision_id,
        "graph_id": str(graph.get("graph_id") or ""),
        "task_graph_json": _task_dag_relative_path(ctx, graph_path),
        "node_id": str(node_id),
        "plan_item_ref": str(graph_node.get("plan_item_ref") or ""),
        "task_id": task_id,
        "change_id": change_id or None,
        "bootstrap_state": "bootstrapped",
        "workflow_boundary": workflow_boundary,
        "promotion_mode": "execution_only_local_first" if local_first_execution_only else "reviewable_remote_change",
        "local_lineage_allowed": local_first_execution_only,
        "remote_change_required": not local_first_execution_only,
        "remote_workflow_allowed": shared_boundary_node,
        "single_path_dag": True,
        "dag_shared_boundary_node": shared_boundary_node,
        "final_remote_disposition_default": _task_dag_final_remote_disposition_default(ctx, graph),
        "graph_run_session_id": graph_run_session_id or None,
    }
    if graph_run_session_id:
        try:
            run_session = remote_get_session(remote_row["url"], graph_run_session_id, repo_name=repo_name)
            run_metadata = run_session.get("metadata") if isinstance(run_session.get("metadata"), dict) else {}
            graph_run_id = str(run_metadata.get("graph_run_id") or "").strip()
            if graph_run_id:
                session_metadata["graph_run_id"] = graph_run_id
        except RemoteError:
            pass

    session = remote_create_session(
        remote_row["url"],
        repo_name,
        "agent_run",
        task_id=task_id,
        change_id=change_id or None,
        title=f"Task DAG node {node_id}: {graph_node.get('title') or template.get('title') or task_id}",
        line_name=session_line_name,
        worktree_name=_normalize_text_value((worktree_payload or {}).get("name")),
        model_name=_effective_model_name(ctx),
        metadata=session_metadata,
    )
    if graph_run_session_id:
        remote_append_session_event(
            remote_row["url"],
            graph_run_session_id,
            "task_graph.node_bootstrapped",
            {
                "node_id": str(node_id),
                "task_id": task_id,
                "change_id": change_id,
                "session_id": session.get("session_id"),
                "worktree_name": (worktree_payload or {}).get("name"),
                "worktree_path": (worktree_payload or {}).get("path"),
            },
            repo_name=repo_name,
        )

    return {
        "node_id": str(node_id),
        "plan_item_ref": graph_node.get("plan_item_ref"),
        "task_id": task_id,
        "change_id": change_id,
        "workflow_boundary": workflow_boundary,
        "created_change": created_change,
        "worktree": worktree_payload,
        "created_worktree": materialized.get("created_worktree"),
        "session": {
            "session_id": session.get("session_id"),
            "session_kind": session.get("session_kind") or session.get("kind") or "agent_run",
            "title": session.get("title"),
            "metadata": session_metadata,
        },
    }
