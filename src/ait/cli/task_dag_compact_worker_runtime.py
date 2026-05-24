from __future__ import annotations

from dataclasses import is_dataclass, replace
import json
import os
import sys
from pathlib import Path
from typing import Any

from ait_protocol.common import normalize_optional_text, utc_now

from ..remote_client import RemoteError, create_session as _fallback_remote_create_session
from ..store import (
    create_snapshot as _fallback_create_snapshot,
    RepoContext,
    current_line as _fallback_current_line,
    get_line as _fallback_get_line,
    load_config as _fallback_load_config,
    workspace_status as _fallback_workspace_status,
)
from ..task_dag_readiness import task_dag_final_remote_disposition_default as _fallback_task_dag_final_remote_disposition_default
from ..workflow_conversation import infer_workflow_context as _fallback_infer_workflow_context
from .remote_ci_readiness_helpers import (
    _remote_error_status_code as _fallback_remote_error_status_code,
    _remote_read_task_dag_readiness as _fallback_remote_read_task_dag_readiness,
)
from .remote_session_wrappers import (
    remote_append_session_event as _fallback_remote_append_session_event,
    remote_close_session as _fallback_remote_close_session,
)
from .reply_runtime_seam import (
    generate_session_reply as _fallback_generate_session_reply,
    load_reply_generation_config as _fallback_load_reply_generation_config,
)
from .runtime_defaults import (
    _effective_author_mode as _fallback_effective_author_mode,
    _effective_model_name as _fallback_effective_model_name,
    _normalize_text_value as _fallback_normalize_text_value,
)
from .session_command_analysis import _normalize_turn_analysis_payload as _fallback_normalize_turn_analysis_payload
from .task_dag_compact_packet_authoring import (
    DEFAULT_TASK_DAG_COMPACT_PACKET_MAX_COMMAND_COUNT,
    DEFAULT_TASK_DAG_COMPACT_PACKET_REPLY_POLL_TIMEOUT_SECONDS,
    DEFAULT_TASK_DAG_IMPLEMENTATION_AUTO_LAND_REPLY_POLL_TIMEOUT_SECONDS,
    DEFAULT_TASK_DAG_IMPLEMENTATION_PACKET_REPLY_POLL_TIMEOUT_SECONDS,
    _task_dag_generate_compact_packet_artifacts as _fallback_task_dag_generate_compact_packet_artifacts,
    _task_dag_load_comparison_evidence as _fallback_task_dag_load_comparison_evidence,
)
from .task_dag_execute_run_controls import _task_dag_refresh_execute_run as _fallback_task_dag_refresh_execute_run
from .task_dag_runtime_helpers import (
    _task_dag_readiness_payload as _fallback_task_dag_readiness_payload,
    _task_dag_readiness_from_remote_inventory as _fallback_task_dag_readiness_from_remote_inventory,
    _task_dag_relative_path as _fallback_task_dag_relative_path,
)
from .task_tracking_bindings import _task_worktree_repo_ctx as _fallback_task_worktree_repo_ctx
from .task_worktree_guidance import _task_worktree_guidance as _fallback_task_worktree_guidance
from .task_worktree_resolution import (
    _find_bound_task_worktree as _fallback_find_bound_task_worktree,
    _session_bound_worktree as _fallback_session_bound_worktree,
)


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is None:
        return fallback
    return getattr(app_module, name, fallback)


def current_line(ctx: RepoContext) -> str:
    return _app_override("current_line", _fallback_current_line)(ctx)


def create_snapshot(ctx: RepoContext, message: str | None, *, parent_snapshot_id: str | None = None) -> dict[str, Any]:
    return _app_override("create_snapshot", _fallback_create_snapshot)(
        ctx,
        message,
        parent_snapshot_id=parent_snapshot_id,
    )


def get_line(ctx: RepoContext, name: str) -> dict[str, Any]:
    return _app_override("get_line", _fallback_get_line)(ctx, name)


def workspace_status(ctx: RepoContext, *, snapshot_id: str | None = None, line_name: str | None = None) -> dict[str, Any]:
    return _app_override("workspace_status", _fallback_workspace_status)(
        ctx,
        snapshot_id=snapshot_id,
        line_name=line_name,
    )


def load_config(ctx: RepoContext) -> dict[str, Any]:
    return _app_override("load_config", _fallback_load_config)(ctx)


def remote_create_session(*args: Any, **kwargs: Any) -> Any:
    return _app_override("remote_create_session", _fallback_remote_create_session)(*args, **kwargs)


def remote_append_session_event(*args: Any, **kwargs: Any) -> Any:
    return _app_override("remote_append_session_event", _fallback_remote_append_session_event)(*args, **kwargs)


def remote_close_session(*args: Any, **kwargs: Any) -> Any:
    return _app_override("remote_close_session", _fallback_remote_close_session)(*args, **kwargs)


def load_reply_generation_config(*args: Any, **kwargs: Any) -> Any:
    return _app_override("load_reply_generation_config", _fallback_load_reply_generation_config)(*args, **kwargs)


def generate_session_reply(*args: Any, **kwargs: Any) -> Any:
    return _app_override("generate_session_reply", _fallback_generate_session_reply)(*args, **kwargs)


def infer_workflow_context(*args: Any, **kwargs: Any) -> Any:
    return _app_override("infer_workflow_context", _fallback_infer_workflow_context)(*args, **kwargs)


def _effective_author_mode(ctx: RepoContext) -> str:
    return _app_override("_effective_author_mode", _fallback_effective_author_mode)(ctx)


def _effective_model_name(ctx: RepoContext) -> str | None:
    return _app_override("_effective_model_name", _fallback_effective_model_name)(ctx)


def _normalize_text_value(value: str | None) -> str | None:
    return _app_override("_normalize_text_value", _fallback_normalize_text_value)(value)


def _normalize_turn_analysis_payload(payload: dict[str, Any] | None) -> dict[str, Any] | None:
    return _app_override("_normalize_turn_analysis_payload", _fallback_normalize_turn_analysis_payload)(payload)


def _task_dag_generate_compact_packet_artifacts(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_generate_compact_packet_artifacts", _fallback_task_dag_generate_compact_packet_artifacts)(*args, **kwargs)


def _task_dag_load_comparison_evidence(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_load_comparison_evidence", _fallback_task_dag_load_comparison_evidence)(*args, **kwargs)


def _task_dag_refresh_execute_run(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_refresh_execute_run", _fallback_task_dag_refresh_execute_run)(*args, **kwargs)


def _remote_read_task_dag_readiness(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_remote_read_task_dag_readiness", _fallback_remote_read_task_dag_readiness)(*args, **kwargs)


def _task_dag_readiness_from_remote_inventory(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_readiness_from_remote_inventory", _fallback_task_dag_readiness_from_remote_inventory)(*args, **kwargs)


def _task_dag_readiness_payload(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_readiness_payload", _fallback_task_dag_readiness_payload)(*args, **kwargs)


def _task_dag_relative_path(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_dag_relative_path", _fallback_task_dag_relative_path)(*args, **kwargs)


def _remote_error_status_code(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_remote_error_status_code", _fallback_remote_error_status_code)(*args, **kwargs)


def _session_bound_worktree(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_session_bound_worktree", _fallback_session_bound_worktree)(*args, **kwargs)


def _find_bound_task_worktree(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_find_bound_task_worktree", _fallback_find_bound_task_worktree)(*args, **kwargs)


def _task_worktree_repo_ctx(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_worktree_repo_ctx", _fallback_task_worktree_repo_ctx)(*args, **kwargs)


def _task_worktree_guidance(*args: Any, **kwargs: Any) -> Any:
    return _app_override("_task_worktree_guidance", _fallback_task_worktree_guidance)(*args, **kwargs)


def _task_dag_auto_continue_profile(*args: Any, **kwargs: Any) -> Any:
    fn = _app_override("_task_dag_auto_continue_profile", None)
    if fn is None:
        raise RuntimeError("ait.cli.app did not expose the compact-DAG auto-continue profile helper.")
    return fn(*args, **kwargs)


def _task_dag_auto_bootstrap_ready_node_ids(*args: Any, **kwargs: Any) -> Any:
    fn = _app_override("_task_dag_auto_bootstrap_ready_node_ids", None)
    if fn is None:
        raise RuntimeError("ait.cli.app did not expose the compact-DAG auto-bootstrap helper.")
    return fn(*args, **kwargs)


def _task_dag_materialize_node_lineage(*args: Any, **kwargs: Any) -> Any:
    fn = _app_override("_task_dag_materialize_node_lineage", None)
    if fn is None:
        raise RuntimeError("ait.cli.app did not expose the compact-DAG node-lineage materializer.")
    return fn(*args, **kwargs)


def _task_dag_compact_worker_focus_lineage(
    compact_packet_surface: dict[str, Any] | None,
    *,
    recorded_run: dict[str, Any] | None = None,
) -> dict[str, str | None]:
    change_focus_policy = (
        compact_packet_surface.get("change_focus_policy")
        if isinstance(compact_packet_surface, dict) and isinstance(compact_packet_surface.get("change_focus_policy"), dict)
        else {}
    )
    next_focus = change_focus_policy.get("next_focus") if isinstance(change_focus_policy.get("next_focus"), dict) else {}
    recorded_metadata = (
        recorded_run.get("metadata")
        if isinstance(recorded_run, dict) and isinstance(recorded_run.get("metadata"), dict)
        else {}
    )
    next_focus_node_id = _normalize_text_value(next_focus.get("node_id"))
    next_focus_task_id = _normalize_text_value(next_focus.get("task_id"))
    next_focus_change_id = _normalize_text_value(next_focus.get("change_id"))
    if next_focus_node_id is not None:
        return {
            "node_id": next_focus_node_id,
            "task_id": next_focus_task_id,
            "change_id": next_focus_change_id,
        }
    return {
        "node_id": _normalize_text_value(recorded_metadata.get("next_focus_node_id")),
        "task_id": (
            next_focus_task_id
            or _normalize_text_value(recorded_metadata.get("next_focus_task_id"))
        ),
        "change_id": (
            next_focus_change_id
            or _normalize_text_value(recorded_metadata.get("next_focus_change_id"))
        ),
    }


def _task_dag_validate_compact_worker_contract(
    graph: dict[str, Any],
    *,
    compact_packet_surface: dict[str, Any] | None,
    worker_metadata: dict[str, Any] | None = None,
) -> None:
    expected_final_remote_disposition_default = bool(_fallback_task_dag_final_remote_disposition_default(graph))

    def _extract_focus(label: str, payload: dict[str, Any] | None) -> dict[str, str | None] | None:
        if not isinstance(payload, dict):
            return None
        next_focus = (
            (payload.get("change_focus_policy") or {}).get("next_focus")
            if isinstance(payload.get("change_focus_policy"), dict)
            else {}
        )
        if not isinstance(next_focus, dict):
            next_focus = {}
        nested_node_id = _normalize_text_value(next_focus.get("node_id"))
        top_node_id = _normalize_text_value(payload.get("next_focus_node_id"))
        nested_task_id = _normalize_text_value(next_focus.get("task_id"))
        top_task_id = _normalize_text_value(payload.get("next_focus_task_id"))
        nested_change_id = _normalize_text_value(next_focus.get("change_id"))
        top_change_id = _normalize_text_value(payload.get("next_focus_change_id"))
        for field_name, nested_value, top_value in (
            ("node_id", nested_node_id, top_node_id),
            ("task_id", nested_task_id, top_task_id),
            ("change_id", nested_change_id, top_change_id),
        ):
            if nested_value is not None and top_value is not None and nested_value != top_value:
                raise ValueError(
                    f"Compact DAG focus metadata drift detected in {label}: "
                    f"`change_focus_policy.next_focus.{field_name}` is `{nested_value}` "
                    f"but top-level `{field_name}` is `{top_value}`."
                )
        final_flag = payload.get("final_remote_disposition_default")
        if final_flag is not None and bool(final_flag) != expected_final_remote_disposition_default:
            raise ValueError(
                f"Compact DAG mode drift detected in {label}: "
                f"`final_remote_disposition_default={bool(final_flag)}` does not match the graph contract "
                f"`{expected_final_remote_disposition_default}`."
            )
        return {
            "label": label,
            "node_id": nested_node_id or top_node_id,
            "task_id": nested_task_id or top_task_id,
            "change_id": nested_change_id or top_change_id,
        }

    focus_sources = [
        focus
        for focus in (
            _extract_focus("compact_packet_surface", compact_packet_surface),
            _extract_focus("worker_metadata", worker_metadata),
        )
        if isinstance(focus, dict)
    ]
    canonical: dict[str, str | None] | None = None
    for focus in focus_sources:
        if canonical is None:
            canonical = focus
            continue
        for field_name in ("node_id", "task_id", "change_id"):
            current_value = _normalize_text_value(focus.get(field_name))
            canonical_value = _normalize_text_value(canonical.get(field_name))
            if current_value is not None and canonical_value is not None and current_value != canonical_value:
                raise ValueError(
                    f"Compact DAG focus metadata drift detected between {canonical['label']} and {focus['label']}: "
                    f"`{field_name}` changed from `{canonical_value}` to `{current_value}`."
                )


def _task_dag_compact_worker_bound_worktree(
    ctx: RepoContext,
    *,
    remote_row: dict[str, Any],
    compact_packet_surface: dict[str, Any] | None,
    recorded_run: dict[str, Any] | None = None,
) -> dict[str, Any] | None:
    focus_lineage = _task_dag_compact_worker_focus_lineage(compact_packet_surface, recorded_run=recorded_run)
    change_id = _normalize_text_value(focus_lineage.get("change_id"))
    task_id = _normalize_text_value(focus_lineage.get("task_id"))
    remote_name = _normalize_text_value((remote_row or {}).get("name")) or _normalize_text_value((remote_row or {}).get("remote_name"))

    def _matches_focus(worktree: dict[str, Any] | None) -> bool:
        if not isinstance(worktree, dict):
            return False
        bound_task_id = _normalize_text_value(worktree.get("bound_task_id")) or _normalize_text_value(worktree.get("task_id"))
        bound_change_id = _normalize_text_value(worktree.get("bound_change_id")) or _normalize_text_value(worktree.get("change_id"))
        if task_id is not None and bound_task_id != task_id:
            return False
        if change_id is not None and bound_change_id != change_id:
            return False
        return task_id is not None or change_id is not None

    resolution_attempts: list[tuple[bool, dict[str, Any]]] = []
    if change_id is not None:
        resolution_attempts.append((True, {"change_id": change_id}))
    if task_id is not None:
        resolution_attempts.append((True, {"task_id": task_id}))
    if remote_name is not None and change_id is not None:
        resolution_attempts.append((False, {"remote_name": remote_name, "change_id": change_id}))
    if remote_name is not None and task_id is not None:
        resolution_attempts.append((False, {"remote_name": remote_name, "task_id": task_id}))
    for use_local, kwargs in resolution_attempts:
        try:
            worktree = _session_bound_worktree(ctx, local=use_local, remote_name=kwargs.pop("remote_name", None), **kwargs)
        except (KeyError, RemoteError, ValueError):
            worktree = None
        if _matches_focus(worktree):
            return worktree

    if task_id is not None:
        focus_worktree = _find_bound_task_worktree(_task_worktree_repo_ctx(ctx), task_id)
        if _matches_focus(focus_worktree):
            return focus_worktree
    return None


def _task_dag_compact_worker_worktree_actual_root(worktree: dict[str, Any] | None) -> str | None:
    if not isinstance(worktree, dict):
        return None
    path_value = (
        _normalize_text_value(worktree.get("path"))
        or _normalize_text_value(worktree.get("workspace_root"))
        or _normalize_text_value(worktree.get("open_path"))
        or _normalize_text_value(worktree.get("alias_path"))
    )
    if path_value is None:
        return None
    return str(Path(path_value).expanduser().resolve())


def _task_dag_compact_worker_bound_worktree_root(
    bound_worktree: dict[str, Any] | None,
    *,
    compact_packet_surface: dict[str, Any] | None,
    command_name: str,
    recorded_run: dict[str, Any] | None = None,
) -> str | None:
    if not isinstance(bound_worktree, dict):
        return None
    target_workspace_root = _task_dag_compact_worker_worktree_actual_root(bound_worktree)
    if target_workspace_root is not None:
        return target_workspace_root
    worktree_name = str(bound_worktree.get("name") or "matching worktree")
    bound_task_id = (
        _normalize_text_value(bound_worktree.get("bound_task_id"))
        or _normalize_text_value(bound_worktree.get("task_id"))
        or _normalize_text_value(_task_dag_compact_worker_focus_lineage(compact_packet_surface, recorded_run=recorded_run).get("task_id"))
    )
    task_fragment = f" for task `{bound_task_id}`" if bound_task_id else ""
    raise ValueError(
        f"`ait {command_name}` requires the bound task worktree for compact DAG implementation authoring. "
        f"Resolved bound worktree `{worktree_name}`{task_fragment} has no usable workspace root."
    )


def _guard_task_dag_implementation_authoring_workspace(
    ctx: RepoContext,
    *,
    remote_row: dict[str, Any],
    compact_packet_surface: dict[str, Any] | None,
    command_name: str,
    recorded_run: dict[str, Any] | None = None,
) -> dict[str, Any]:
    worktree = _task_dag_compact_worker_bound_worktree(
        ctx,
        remote_row=remote_row,
        compact_packet_surface=compact_packet_surface,
        recorded_run=recorded_run,
    )
    workspace_root = str(ctx.root.resolve())
    if not isinstance(worktree, dict):
        return {
            "worktree": None,
            "guidance": None,
            "workspace_root": workspace_root,
        }
    target_workspace_root = _task_dag_compact_worker_bound_worktree_root(
        worktree,
        compact_packet_surface=compact_packet_surface,
        command_name=command_name,
        recorded_run=recorded_run,
    )
    guidance = _task_worktree_guidance(ctx, worktree)
    if workspace_root != target_workspace_root:
        worktree_name = str(worktree.get("name") or "matching worktree")
        bound_task_id = (
            _normalize_text_value(worktree.get("bound_task_id"))
            or _normalize_text_value(worktree.get("task_id"))
            or _normalize_text_value(_task_dag_compact_worker_focus_lineage(compact_packet_surface, recorded_run=recorded_run).get("task_id"))
        )
        cd_command = (
            _normalize_text_value((guidance or {}).get("cd_command"))
            or _normalize_text_value(worktree.get("cd_command"))
            or f"cd {target_workspace_root}"
        )
        task_fragment = f" for task `{bound_task_id}`" if bound_task_id else ""
        raise ValueError(
            f"`ait {command_name}` requires the bound task worktree for compact DAG implementation authoring. "
            f"Continue in bound worktree `{worktree_name}`{task_fragment} with `{cd_command}`."
        )
    return {
        "worktree": worktree,
        "guidance": guidance,
        "workspace_root": target_workspace_root,
    }


def _task_dag_compact_worker_reply_excerpt(reply_text: str | None, *, limit: int = 280) -> str | None:
    text = " ".join(str(reply_text or "").split()).strip()
    if not text:
        return None
    if len(text) <= limit:
        return text
    return text[: max(limit - 1, 0)].rstrip() + "…"


def _task_dag_compact_worker_focus_metadata(worker_metadata: dict[str, Any]) -> dict[str, str | None]:
    change_focus_policy = (
        worker_metadata.get("change_focus_policy")
        if isinstance(worker_metadata.get("change_focus_policy"), dict)
        else {}
    )
    next_focus = change_focus_policy.get("next_focus") if isinstance(change_focus_policy.get("next_focus"), dict) else {}
    return {
        "node_id": _normalize_text_value(next_focus.get("node_id")) or _normalize_text_value(worker_metadata.get("next_focus_node_id")),
        "task_id": _normalize_text_value(next_focus.get("task_id")) or _normalize_text_value(worker_metadata.get("next_focus_task_id")),
        "change_id": _normalize_text_value(next_focus.get("change_id")) or _normalize_text_value(worker_metadata.get("next_focus_change_id")),
    }


def _task_dag_seed_compact_worker_focus_lineage(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    graph: dict[str, Any],
    graph_path: Path,
    recorded_run: dict[str, Any],
    compact_packet_surface: dict[str, Any],
) -> tuple[dict[str, Any], dict[str, Any] | None]:
    next_focus = (
        (compact_packet_surface.get("change_focus_policy") or {}).get("next_focus")
        if isinstance(compact_packet_surface.get("change_focus_policy"), dict)
        else {}
    )
    if not isinstance(next_focus, dict):
        next_focus = {}
    focus_node_id = _normalize_text_value(next_focus.get("node_id"))
    focus_lineage = _task_dag_compact_worker_focus_lineage(compact_packet_surface, recorded_run=recorded_run)
    if focus_node_id is not None and (
        _normalize_text_value(focus_lineage.get("task_id")) or _normalize_text_value(focus_lineage.get("change_id"))
    ):
        existing_worktree = _task_dag_compact_worker_bound_worktree(
            ctx,
            remote_row=remote_row,
            compact_packet_surface=compact_packet_surface,
            recorded_run=recorded_run,
        )
        if isinstance(existing_worktree, dict):
            return compact_packet_surface, existing_worktree
    readiness = _task_dag_readiness_payload(
        ctx,
        graph,
        remote_row.get("name") if isinstance(remote_row, dict) else None,
    )
    if focus_node_id is None:
        profile = _task_dag_auto_continue_profile(ctx, graph)
        candidate_node_ids = _task_dag_auto_bootstrap_ready_node_ids(graph, readiness, profile)
        focus_node_id = _normalize_text_value(candidate_node_ids[0]) if candidate_node_ids else None
    if focus_node_id is None:
        return compact_packet_surface, None
    focus_node = next(
        (
            node
            for node in (graph.get("nodes") or [])
            if isinstance(node, dict) and str(node.get("node_id") or "").strip() == str(focus_node_id)
        ),
        None,
    )
    if not isinstance(focus_node, dict) or str(focus_node.get("node_kind") or "") != "task":
        return compact_packet_surface, None
    materialized = _task_dag_materialize_node_lineage(
        ctx=ctx,
        remote_row=remote_row,
        repo_name=repo_name,
        plan_id=plan_id,
        graph=graph,
        readiness=readiness,
        node_id=focus_node_id,
        create_worktree=True,
        allow_execution_only_without_change=True,
    )
    seeded_task_id = _normalize_text_value(materialized.get("task_id"))
    seeded_change_id = _normalize_text_value(materialized.get("change_id"))
    updated_focus = {
        "focus_unit": "change" if seeded_change_id else "task" if seeded_task_id else "node",
        "node_id": focus_node_id,
        "task_id": seeded_task_id,
        "change_id": seeded_change_id,
    }
    change_focus_policy = (
        compact_packet_surface.get("change_focus_policy")
        if isinstance(compact_packet_surface.get("change_focus_policy"), dict)
        else {}
    )
    focus_queue = change_focus_policy.get("focus_queue") if isinstance(change_focus_policy.get("focus_queue"), list) else []
    updated_focus_queue: list[dict[str, Any]] = []
    replaced = False
    for entry in focus_queue:
        if not isinstance(entry, dict):
            continue
        if str(entry.get("node_id") or "").strip() == focus_node_id:
            merged = dict(entry)
            merged.update({k: v for k, v in updated_focus.items() if v is not None})
            updated_focus_queue.append(merged)
            replaced = True
        else:
            updated_focus_queue.append(dict(entry))
    if not replaced:
        updated_focus_queue.insert(0, {k: v for k, v in updated_focus.items() if v is not None})
    updated_surface = dict(compact_packet_surface)
    updated_surface["change_focus_policy"] = {
        **change_focus_policy,
        "focus_queue": updated_focus_queue,
        "next_focus": updated_focus,
    }
    updated_surface["next_focus_node_id"] = focus_node_id
    updated_surface["next_focus_task_id"] = seeded_task_id
    updated_surface["next_focus_change_id"] = seeded_change_id
    return updated_surface, materialized.get("worktree") if isinstance(materialized.get("worktree"), dict) else None


def _task_dag_compact_worker_local_progress_payload(
    reply_text: str | None,
    *,
    worker_metadata: dict[str, Any],
    worker_session_id: str,
    reply_excerpt: str | None = None,
) -> dict[str, Any] | None:
    progress_line = next(
        (
            line.strip()
            for line in reversed(str(reply_text or "").splitlines())
            if line.strip().startswith("task_dag_local_progress=")
        ),
        None,
    )
    if progress_line is None:
        return None
    raw_payload = progress_line.split("=", 1)[1].strip()
    try:
        parsed = json.loads(raw_payload)
    except json.JSONDecodeError:
        return None
    if isinstance(parsed, dict) and isinstance(parsed.get("task_dag_local_progress"), dict):
        parsed = parsed.get("task_dag_local_progress")
    if not isinstance(parsed, dict):
        return None
    focus = _task_dag_compact_worker_focus_metadata(worker_metadata)
    node_id = _normalize_text_value(parsed.get("node_id")) or focus.get("node_id")
    if node_id is None:
        return None
    status = (_normalize_text_value(parsed.get("status")) or "running").lower()
    if status not in {"running", "completed", "blocked", "failed"}:
        status = "running"
    tests = [str(value).strip() for value in parsed.get("tests") or [] if str(value).strip()]
    summary = _normalize_text_value(parsed.get("summary")) or _normalize_text_value(parsed.get("reason"))
    payload: dict[str, Any] = {
        "node_id": node_id,
        "status": status,
        "task_id": _normalize_text_value(parsed.get("task_id")) or focus.get("task_id"),
        "change_id": _normalize_text_value(parsed.get("change_id")) or focus.get("change_id"),
        "worker_session_id": worker_session_id,
        "worker_session_policy": "task_dag_compact_packet_worker",
    }
    if summary:
        payload["summary"] = summary
    if tests:
        payload["tests"] = tests
    if reply_excerpt:
        payload["reply_excerpt"] = reply_excerpt
    return payload


def _task_dag_compact_worker_completion_snapshot_evidence(
    ctx: RepoContext,
    *,
    remote_row: dict[str, Any],
    compact_packet_surface: dict[str, Any] | None,
    recorded_run: dict[str, Any] | None = None,
    node_id: str | None = None,
) -> dict[str, Any]:
    worktree = _task_dag_compact_worker_bound_worktree(
        ctx,
        remote_row=remote_row,
        compact_packet_surface=compact_packet_surface,
        recorded_run=recorded_run,
    )
    if not isinstance(worktree, dict):
        return {}
    worktree_root = _task_dag_compact_worker_worktree_actual_root(worktree)
    if worktree_root is None:
        return {}
    worktree_ctx = RepoContext.discover(Path(worktree_root))
    line_name = _normalize_text_value(worktree.get("current_line")) or current_line(worktree_ctx)
    try:
        line_row = get_line(worktree_ctx, line_name) if line_name else {}
    except Exception:
        line_row = {}
    baseline_snapshot_id = _normalize_text_value((line_row or {}).get("head_snapshot_id"))
    try:
        delta = workspace_status(worktree_ctx, snapshot_id=baseline_snapshot_id)
    except Exception:
        delta = {}
    if not bool(delta.get("clean")):
        snapshot = create_snapshot(
            worktree_ctx,
            f"Compact DAG completion evidence for node {node_id or 'unknown'}",
            parent_snapshot_id=baseline_snapshot_id,
        )
        baseline_snapshot_id = _normalize_text_value(snapshot.get("snapshot_id")) or baseline_snapshot_id
        line_row = {"head_snapshot_id": baseline_snapshot_id}
    payload: dict[str, Any] = {}
    completion_snapshot_id = _normalize_text_value((line_row or {}).get("head_snapshot_id")) or baseline_snapshot_id
    completion_fork_snapshot_id = _normalize_text_value(worktree.get("fork_snapshot_id"))
    worktree_name = _normalize_text_value(worktree.get("name"))
    if completion_snapshot_id:
        payload["completion_snapshot_id"] = completion_snapshot_id
    if completion_fork_snapshot_id:
        payload["completion_fork_snapshot_id"] = completion_fork_snapshot_id
    if line_name:
        payload["completion_line_name"] = line_name
    if worktree_name:
        payload["completion_worktree_name"] = worktree_name
    return payload


def _task_dag_compact_worker_reply_poll_timeout_seconds(packet_bundle: dict[str, Any]) -> float:
    execution_mode = normalize_optional_text(packet_bundle.get("execution_mode")) or "benchmark"
    if execution_mode != "implementation":
        return float(DEFAULT_TASK_DAG_COMPACT_PACKET_REPLY_POLL_TIMEOUT_SECONDS)
    if bool(packet_bundle.get("final_remote_disposition_default")):
        return float(DEFAULT_TASK_DAG_IMPLEMENTATION_AUTO_LAND_REPLY_POLL_TIMEOUT_SECONDS)
    return float(DEFAULT_TASK_DAG_IMPLEMENTATION_PACKET_REPLY_POLL_TIMEOUT_SECONDS)


def _task_dag_compact_worker_surface_from_snapshot(
    compact_packet_surface: dict[str, Any],
    snapshot: dict[str, Any] | None,
) -> dict[str, Any]:
    updated_surface = dict(compact_packet_surface)
    snapshot_payload = snapshot if isinstance(snapshot, dict) else {}
    change_focus_policy = (
        dict(snapshot_payload.get("change_focus_policy"))
        if isinstance(snapshot_payload.get("change_focus_policy"), dict)
        else {}
    )
    updated_surface["change_focus_policy"] = change_focus_policy
    next_focus = change_focus_policy.get("next_focus") if isinstance(change_focus_policy.get("next_focus"), dict) else {}
    updated_surface["next_focus_node_id"] = _normalize_text_value(
        next_focus.get("node_id") if isinstance(next_focus, dict) else snapshot_payload.get("next_focus_node_id")
    )
    updated_surface["next_focus_task_id"] = _normalize_text_value(
        next_focus.get("task_id") if isinstance(next_focus, dict) else snapshot_payload.get("next_focus_task_id")
    )
    updated_surface["next_focus_change_id"] = _normalize_text_value(
        next_focus.get("change_id") if isinstance(next_focus, dict) else snapshot_payload.get("next_focus_change_id")
    )
    updated_surface["final_remote_disposition_default"] = bool(
        snapshot_payload.get("final_remote_disposition_default", compact_packet_surface.get("final_remote_disposition_default"))
    )
    for key in ("execution_only_node_ids", "converged_output_node_ids", "safety_boundary_node_ids"):
        if key in snapshot_payload:
            updated_surface[key] = [str(value) for value in snapshot_payload.get(key) or [] if str(value)]
    return updated_surface


def _task_dag_prepare_auto_compact_worker_iteration(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    graph: dict[str, Any],
    graph_path: Path,
    recorded_run: dict[str, Any],
    compact_packet_surface: dict[str, Any],
    comparison_evidence: dict[str, Any] | None,
    graph_run_session_id: str,
    graph_run_id: str,
) -> dict[str, Any]:
    current_surface = dict(compact_packet_surface)
    _task_dag_validate_compact_worker_contract(
        graph,
        compact_packet_surface=current_surface,
    )
    if comparison_evidence is None:
        seeded_worktree = None
        current_surface, seeded_worktree = _task_dag_seed_compact_worker_focus_lineage(
            ctx=ctx,
            remote_row=remote_row,
            repo_name=repo_name,
            plan_id=plan_id,
            graph=graph,
            graph_path=graph_path,
            recorded_run=recorded_run,
            compact_packet_surface=current_surface,
        )
        bound_worktree = _task_dag_compact_worker_bound_worktree(
            ctx,
            remote_row=remote_row,
            compact_packet_surface=current_surface,
            recorded_run=recorded_run,
        )
        if bound_worktree is None and isinstance(seeded_worktree, dict):
            bound_worktree = seeded_worktree
        if isinstance(bound_worktree, dict):
            authoring_workspace_root = _task_dag_compact_worker_bound_worktree_root(
                bound_worktree,
                compact_packet_surface=current_surface,
                command_name="plan execute --auto-compact-worker",
                recorded_run=recorded_run,
            ) or str(ctx.root.resolve())
        else:
            focus_lineage = _task_dag_compact_worker_focus_lineage(current_surface, recorded_run=recorded_run)
            focus_node_id = _normalize_text_value(
                ((current_surface.get("change_focus_policy") or {}).get("next_focus") or {}).get("node_id")
            )
            execution_only_node_ids = {
                str(node_id).strip()
                for node_id in (
                    current_surface.get("execution_only_node_ids")
                    or (
                        recorded_run.get("metadata")
                        if isinstance(recorded_run.get("metadata"), dict)
                        else {}
                    ).get("execution_only_node_ids")
                    or []
                )
                if str(node_id).strip()
            }
            if (
                focus_node_id in execution_only_node_ids
                or _normalize_text_value(focus_lineage.get("task_id"))
                or _normalize_text_value(focus_lineage.get("change_id"))
            ):
                raise ValueError(
                    "Compact DAG auto worker could not resolve a seeded focus bound worktree for implementation authoring."
                )
            authoring_workspace = _guard_task_dag_implementation_authoring_workspace(
                ctx,
                remote_row=remote_row,
                compact_packet_surface=current_surface,
                command_name="plan execute --auto-compact-worker",
                recorded_run=recorded_run,
            )
            authoring_workspace_root = str(authoring_workspace.get("workspace_root") or ctx.root.resolve())
    else:
        authoring_workspace_root = str(ctx.root.resolve())

    final_remote_disposition_default = bool(
        current_surface.get("final_remote_disposition_default")
        or (
            recorded_run.get("metadata")
            if isinstance(recorded_run.get("metadata"), dict)
            else {}
        ).get("final_remote_disposition_default")
        or (
            recorded_run.get("metadata")
            if isinstance(recorded_run.get("metadata"), dict)
            else {}
        ).get("auto_land_supported")
    )
    packet_bundle = _task_dag_generate_compact_packet_artifacts(
        ctx,
        plan_id=plan_id,
        graph=graph,
        graph_path=graph_path,
        compact_packet_surface=current_surface,
        graph_run_session_id=graph_run_session_id,
        final_remote_disposition_default=final_remote_disposition_default,
        comparison_evidence=comparison_evidence,
        authoring_workspace_root=authoring_workspace_root,
    )
    worker_workspace_root = (
        authoring_workspace_root
        if comparison_evidence is None
        else str(packet_bundle.get("worker_workspace_root") or ctx.root.resolve())
    )
    worker_repo_root = (
        authoring_workspace_root
        if comparison_evidence is None
        else str(packet_bundle.get("worker_repo_root") or ctx.root.resolve())
    )
    graph_id = str(graph.get("graph_id") or "")
    graph_artifact_path = _task_dag_relative_path(ctx, graph_path)
    reply_poll_timeout_seconds = _task_dag_compact_worker_reply_poll_timeout_seconds(packet_bundle)
    packet_generated_payload = {
        "graph_run_id": graph_run_id,
        "graph_id": graph_id,
        "graph_artifact_path": graph_artifact_path,
        "surface_id": current_surface.get("surface_id"),
        "final_remote_disposition_default": final_remote_disposition_default,
        "packet_available": packet_bundle.get("packet_available"),
        "packet_artifact_path": packet_bundle.get("packet_artifact_path"),
        "surface_artifact_path": packet_bundle.get("surface_artifact_path"),
        "turn_artifact_path": packet_bundle.get("turn_artifact_path"),
        "packet_root_path": packet_bundle.get("packet_root_path"),
        "packet_root_manifest_path": packet_bundle.get("packet_root_manifest_path"),
        "comparison_inputs_packaged": packet_bundle.get("comparison_inputs_packaged"),
        "comparison_evidence_artifact_path": packet_bundle.get("comparison_evidence_artifact_path"),
        "packet_prompt_digest": packet_bundle.get("packet_prompt_digest"),
        "packet_context_digest": packet_bundle.get("packet_context_digest"),
        "reply_poll_timeout_seconds": reply_poll_timeout_seconds,
        "change_focus_policy": current_surface.get("change_focus_policy") or {},
        "next_focus_node_id": (
            (current_surface.get("change_focus_policy") or {}).get("next_focus") or {}
        ).get("node_id")
        if isinstance((current_surface.get("change_focus_policy") or {}).get("next_focus"), dict)
        else None,
        "next_focus_task_id": (
            (current_surface.get("change_focus_policy") or {}).get("next_focus") or {}
        ).get("task_id")
        if isinstance((current_surface.get("change_focus_policy") or {}).get("next_focus"), dict)
        else None,
        "next_focus_change_id": (
            (current_surface.get("change_focus_policy") or {}).get("next_focus") or {}
        ).get("change_id")
        if isinstance((current_surface.get("change_focus_policy") or {}).get("next_focus"), dict)
        else None,
    }
    if graph_run_session_id:
        remote_append_session_event(
            remote_row["url"],
            graph_run_session_id,
            "task_graph.compact_packet_generated",
            packet_generated_payload,
            repo_name=repo_name,
        )
    worker_worktree = (
        None
        if comparison_evidence is not None
        else _task_dag_compact_worker_bound_worktree(
            ctx,
            remote_row=remote_row,
            compact_packet_surface=current_surface,
            recorded_run=recorded_run,
        )
    )
    worker_metadata: dict[str, Any] = {
        "author_mode": _effective_author_mode(ctx),
        "session_policy": "task_dag_compact_packet_worker",
        "plan_id": plan_id,
        "graph_id": graph_id,
        "graph_run_id": graph_run_id,
        "graph_run_session_id": graph_run_session_id,
        "task_graph_json": graph_artifact_path,
        "packet_artifact_path": packet_bundle.get("packet_artifact_path"),
        "packet_surface_artifact_path": packet_bundle.get("surface_artifact_path"),
        "packet_turn_artifact_path": packet_bundle.get("turn_artifact_path"),
        "packet_available": packet_bundle.get("packet_available"),
        "packet_prompt_digest": packet_bundle.get("packet_prompt_digest"),
        "packet_context_digest": packet_bundle.get("packet_context_digest"),
        "packet_root_path": packet_bundle.get("packet_root_path"),
        "packet_root_manifest_path": packet_bundle.get("packet_root_manifest_path"),
        "comparison_inputs_packaged": packet_bundle.get("comparison_inputs_packaged"),
        "comparison_evidence_artifact_path": packet_bundle.get("comparison_evidence_artifact_path"),
        "packet_root_policy": packet_bundle.get("packet_root_policy") or {},
        "execution_mode": packet_bundle.get("execution_mode"),
        "final_remote_disposition_default": final_remote_disposition_default,
        "reply_poll_timeout_seconds": reply_poll_timeout_seconds,
        "workspace_root": worker_workspace_root,
        "repo_root": worker_repo_root,
        "fresh_worker_session": True,
        "worker_session_count": 1,
        "physical_fanout": False,
        "coordinator_plus_worker_end_to_end": False,
        "repo_replay_policy": (
            "graph_scoped_authoring_workspace"
            if packet_bundle.get("execution_mode") == "implementation"
            else "packet_scoped_only"
        ),
        "change_focus_policy": current_surface.get("change_focus_policy") or {},
        "next_focus_node_id": (
            (current_surface.get("change_focus_policy") or {}).get("next_focus") or {}
        ).get("node_id")
        if isinstance((current_surface.get("change_focus_policy") or {}).get("next_focus"), dict)
        else None,
        "next_focus_task_id": (
            (current_surface.get("change_focus_policy") or {}).get("next_focus") or {}
        ).get("task_id")
        if isinstance((current_surface.get("change_focus_policy") or {}).get("next_focus"), dict)
        else None,
        "next_focus_change_id": (
            (current_surface.get("change_focus_policy") or {}).get("next_focus") or {}
        ).get("change_id")
        if isinstance((current_surface.get("change_focus_policy") or {}).get("next_focus"), dict)
        else None,
        "compact_packet_surface": current_surface,
    }
    _task_dag_validate_compact_worker_contract(
        graph,
        compact_packet_surface=current_surface,
        worker_metadata=worker_metadata,
    )
    return {
        "compact_packet_surface": current_surface,
        "packet_bundle": packet_bundle,
        "final_remote_disposition_default": final_remote_disposition_default,
        "worker_worktree": worker_worktree,
        "worker_worktree_name": _normalize_text_value((worker_worktree or {}).get("name")),
        "worker_workspace_root": worker_workspace_root,
        "worker_repo_root": worker_repo_root,
        "reply_poll_timeout_seconds": reply_poll_timeout_seconds,
        "worker_metadata": worker_metadata,
        "turn_text": str(packet_bundle.get("turn_text") or ""),
    }


def _task_dag_compact_worker_command_examples(turn_analysis: dict[str, Any] | None) -> list[str]:
    normalized = _normalize_turn_analysis_payload(turn_analysis) or {}
    seen: set[str] = set()
    commands: list[str] = []
    for raw in normalized.get("commands") or []:
        text = str(raw).strip()
        if text and text not in seen:
            seen.add(text)
            commands.append(text)
    for row in normalized.get("top_commands") or []:
        if not isinstance(row, dict):
            continue
        text = (
            str(row.get("example") or row.get("command") or row.get("signature") or row.get("name") or "").strip()
        )
        if text and text not in seen:
            seen.add(text)
            commands.append(text)
    return commands


def _task_dag_compact_worker_boundary_report(
    packet_root_policy: dict[str, Any],
    turn_analysis: dict[str, Any] | None,
) -> dict[str, Any]:
    normalized = _normalize_turn_analysis_payload(turn_analysis) or {
        "command_count": 0,
        "distinct_command_count": 0,
        "commands": [],
        "top_commands": [],
        "optimization_hints": [],
        "optimization_summary": "",
    }
    forbidden_patterns = [str(row).strip() for row in packet_root_policy.get("forbidden_command_patterns") or [] if str(row).strip()]
    max_command_count = int(packet_root_policy.get("max_command_count") or DEFAULT_TASK_DAG_COMPACT_PACKET_MAX_COMMAND_COUNT)
    commands = _task_dag_compact_worker_command_examples(normalized)
    violations: list[dict[str, Any]] = []
    if int(normalized.get("command_count") or 0) > max_command_count:
        violations.append(
            {
                "code": "command_count_over_budget",
                "message": f"command_count {int(normalized.get('command_count') or 0)} exceeds budget {max_command_count}",
            }
        )
    for command in commands:
        lowered = command.lower()
        for pattern in forbidden_patterns:
            if pattern.lower() in lowered:
                violations.append(
                    {
                        "code": "forbidden_probe_pattern",
                        "pattern": pattern,
                        "command": command,
                    }
                )
                break
    return {
        "execution_mode": packet_root_policy.get("execution_mode"),
        "policy_strength": packet_root_policy.get("policy_strength"),
        "packet_root_path": packet_root_policy.get("packet_root_path"),
        "packet_root_manifest_path": packet_root_policy.get("packet_root_manifest_path"),
        "authoring_workspace_root": packet_root_policy.get("authoring_workspace_root"),
        "allowed_path_prefixes": packet_root_policy.get("allowed_path_prefixes") or [],
        "allowed_file_hints": packet_root_policy.get("allowed_file_hints") or [],
        "forbidden_command_patterns": forbidden_patterns,
        "max_command_count": max_command_count,
        "command_count": int(normalized.get("command_count") or 0),
        "distinct_command_count": int(normalized.get("distinct_command_count") or 0),
        "inspected_commands": commands,
        "violations": violations,
        "status": "ok" if not violations else "violated",
    }


def _task_dag_compact_worker_turn_outcome(
    *,
    turn_result: dict[str, Any] | None = None,
    assistant_event: dict[str, Any] | None = None,
) -> dict[str, Any]:
    payload = assistant_event.get("payload") if isinstance(assistant_event, dict) and isinstance(assistant_event.get("payload"), dict) else {}
    direct = turn_result or {}
    return {
        "reply_text": direct.get("reply_text") or payload.get("text"),
        "turn_analysis": direct.get("turn_analysis") or payload.get("turn_analysis") or {},
        "usage": payload.get("usage") or {},
        "model": payload.get("model") or direct.get("model"),
        "response_id": payload.get("response_id") or direct.get("response_id"),
        "assistant_reply_sequence": int((assistant_event or {}).get("sequence") or 0),
    }


def _task_dag_local_reply_generation_config(
    *,
    repo_name: str,
    repo_root: Path,
):
    reply_env = dict(os.environ)
    reply_env.pop("AIT_REPO_NAME", None)
    reply_env.pop("AIT_TELEGRAM_REPO_NAME", None)
    reply_env.pop("AIT_TELEGRAM_ENV_PATH", None)
    reply_env.pop("AIT_CHAT_ENV_PATH", None)
    config = load_reply_generation_config(
        repo_name=repo_name,
        repo_root=repo_root,
        env=reply_env,
    )
    # Compact DAG worker turns are packet-scoped and may hand off between
    # different bound worktrees while remaining one logical DAG worker session.
    # Reusing a persistent Codex app-server thread across those worktree-local
    # repo roots can strand the next focus turn before its completion is written
    # back into the graph-run. Keep DAG worker turns on fresh Codex clients so
    # cross-worktree continuation stays DAG-only and does not change non-DAG
    # session/runtime behavior.
    if is_dataclass(config):
        try:
            return replace(config, codex_persistent_client=False)
        except TypeError:
            pass
    if hasattr(config, "codex_persistent_client"):
        try:
            setattr(config, "codex_persistent_client", False)
        except Exception:
            pass
    return config


def _task_dag_run_local_compact_worker_turn(
    *,
    remote_row: dict[str, Any],
    repo_name: str,
    worker_session: dict[str, Any],
    worker_session_id: str,
    worker_title: str,
    worker_metadata: dict[str, Any],
    turn_text: str,
    repo_root: Path,
) -> dict[str, Any]:
    session_payload = {
        **(worker_session if isinstance(worker_session, dict) else {}),
        "session_id": worker_session_id,
        "session_kind": (
            (worker_session.get("session_kind") if isinstance(worker_session, dict) else None)
            or (worker_session.get("kind") if isinstance(worker_session, dict) else None)
            or "agent_run"
        ),
        "repo_name": repo_name,
        "title": (
            (worker_session.get("title") if isinstance(worker_session, dict) else None)
            or worker_title
        ),
        "metadata": dict(worker_metadata),
    }
    workflow_context = infer_workflow_context(text=turn_text, session=session_payload)
    user_payload: dict[str, Any] = {
        "source": "task_dag_compact_packet",
        "surface_title": worker_title,
        "text": turn_text,
        "ingested_at": str(utc_now()),
    }
    if workflow_context:
        user_payload["workflow_context"] = workflow_context
    user_event = remote_append_session_event(
        remote_row["url"],
        worker_session_id,
        "session.message",
        user_payload,
        repo_name=repo_name,
    )
    user_sequence = int((user_event if isinstance(user_event, dict) else {}).get("sequence") or 1)
    normalized_user_event = {
        "sequence": user_sequence,
        "event_type": "session.message",
        "payload": user_payload,
    }
    reply_config = _task_dag_local_reply_generation_config(repo_name=repo_name, repo_root=repo_root)
    reply = generate_session_reply(
        reply_config,
        session=session_payload,
        events=[normalized_user_event],
        chat_id=worker_session_id,
        chat_title=worker_title,
        checkpoint=None,
        surface="task_dag_compact_packet",
    )
    assistant_payload = {
        "source": reply.source,
        "generated_via": "ait_cli_local_compact_worker",
        "text": reply.text,
        "turn_analysis": reply.turn_analysis or {},
        "model": reply.model,
        "response_id": reply.response_id,
        "usage": reply.usage or {},
        "reply_to_sequence": user_sequence,
        "delivered_via": "local_compact_worker",
        "session_surface": "task_dag_compact_packet",
        "surface_title": worker_title,
        "generated_at": str(utc_now()),
    }
    assistant_event = remote_append_session_event(
        remote_row["url"],
        worker_session_id,
        "assistant.reply",
        assistant_payload,
        repo_name=repo_name,
    )
    assistant_sequence = int((assistant_event if isinstance(assistant_event, dict) else {}).get("sequence") or (user_sequence + 1))
    normalized_assistant_event = {
        **(assistant_event if isinstance(assistant_event, dict) else {}),
        "sequence": assistant_sequence,
        "event_type": "assistant.reply",
        "payload": assistant_payload,
    }
    return {
        "ok": True,
        "session_id": worker_session_id,
        "surface": "task_dag_compact_packet",
        "reply_text": reply.text,
        "turn_analysis": reply.turn_analysis or {},
        "assistant_event": normalized_assistant_event,
        "model": reply.model,
        "response_id": reply.response_id,
        "usage": reply.usage or {},
    }


def _task_dag_start_auto_compact_worker(
    *,
    ctx: RepoContext,
    remote_row: dict[str, Any],
    repo_name: str,
    plan_id: str,
    graph: dict[str, Any],
    graph_path: Path,
    recorded_run: dict[str, Any],
    compact_packet_surface: dict[str, Any],
    comparison_evidence_report: Path | None = None,
    comparison_evidence_workload_id: str | None = None,
) -> dict[str, Any]:
    if not compact_packet_surface:
        raise ValueError("Auto compact worker requires compact packet surface guidance.")
    compact_packet_surface = dict(compact_packet_surface)
    graph_run_session_id = str(recorded_run.get("session_id") or "").strip()
    graph_run_id = str(recorded_run.get("graph_run_id") or "").strip()
    comparison_evidence = None
    if comparison_evidence_report is not None:
        comparison_evidence = _task_dag_load_comparison_evidence(
            ctx,
            graph=graph,
            report_path=comparison_evidence_report,
            workload_id=comparison_evidence_workload_id,
        )
    graph_id = str(graph.get("graph_id") or "")
    graph_artifact_path = _task_dag_relative_path(ctx, graph_path)
    iteration = _task_dag_prepare_auto_compact_worker_iteration(
        ctx=ctx,
        remote_row=remote_row,
        repo_name=repo_name,
        plan_id=plan_id,
        graph=graph,
        graph_path=graph_path,
        recorded_run=recorded_run,
        compact_packet_surface=compact_packet_surface,
        comparison_evidence=comparison_evidence,
        graph_run_session_id=graph_run_session_id,
        graph_run_id=graph_run_id,
    )
    compact_packet_surface = iteration["compact_packet_surface"]
    packet_bundle = iteration["packet_bundle"]
    final_remote_disposition_default = bool(iteration["final_remote_disposition_default"])
    worker_workspace_root = str(iteration["worker_workspace_root"])
    worker_repo_root = str(iteration["worker_repo_root"])
    reply_poll_timeout_seconds = float(iteration["reply_poll_timeout_seconds"])
    worker_worktree_name = iteration["worker_worktree_name"]
    worker_metadata = dict(iteration["worker_metadata"])
    worker_title = f"Compact DAG worker: {graph_id or plan_id}"
    worker_session = remote_create_session(
        remote_row["url"],
        repo_name,
        "agent_run",
        title=worker_title,
        line_name=current_line(ctx),
        worktree_name=worker_worktree_name,
        model_name=_effective_model_name(ctx),
        metadata=worker_metadata,
    )
    worker_session_id = str(worker_session.get("session_id") or "")
    turn_delivery_status = "local_cli_reply_generation"
    outcome = None
    boundary_report = None
    local_progress_payload = None
    post_worker_run = None
    graph_run_close_out = None
    worker_session_close_out = None
    continuation_count = 0
    continued_focus_node_ids: list[str] = []
    auto_continue_supported = bool(_task_dag_auto_continue_profile(ctx, graph).get("auto_continue_supported"))
    max_iterations = max(1, len(graph.get("nodes") or [])) + 1
    for _ in range(max_iterations):
        worker_session_payload = {
            **worker_session,
            "worktree_name": iteration.get("worker_worktree_name"),
            "metadata": dict(worker_metadata),
        }
        turn_result = _task_dag_run_local_compact_worker_turn(
            remote_row=remote_row,
            repo_name=repo_name,
            worker_session=worker_session_payload,
            worker_session_id=worker_session_id,
            worker_title=worker_title,
            worker_metadata=worker_metadata,
            turn_text=str(iteration.get("turn_text") or ""),
            repo_root=Path(worker_repo_root).expanduser(),
        )
        if not bool((turn_result or {}).get("ok", True)):
            raise ValueError(
                str((turn_result or {}).get("error") or f"Compact worker local reply failed for {worker_session_id}.")
            )
        assistant_event = (turn_result or {}).get("assistant_event") if isinstance((turn_result or {}).get("assistant_event"), dict) else None
        outcome = _task_dag_compact_worker_turn_outcome(turn_result=turn_result, assistant_event=assistant_event)
        boundary_report = _task_dag_compact_worker_boundary_report(
            packet_bundle.get("packet_root_policy") if isinstance(packet_bundle.get("packet_root_policy"), dict) else {},
            outcome.get("turn_analysis") if isinstance(outcome.get("turn_analysis"), dict) else {},
        )
        reply_excerpt = _task_dag_compact_worker_reply_excerpt(outcome.get("reply_text"))
        focus_metadata = _task_dag_compact_worker_focus_metadata(worker_metadata)
        local_progress_payload = _task_dag_compact_worker_local_progress_payload(
            outcome.get("reply_text"),
            worker_metadata=worker_metadata,
            worker_session_id=worker_session_id,
            reply_excerpt=reply_excerpt,
        )
        if (
            isinstance(local_progress_payload, dict)
            and str(local_progress_payload.get("status") or "").strip().lower() == "completed"
        ):
            local_progress_payload = {
                **local_progress_payload,
                **_task_dag_compact_worker_completion_snapshot_evidence(
                    ctx,
                    remote_row=remote_row,
                    compact_packet_surface=compact_packet_surface,
                    recorded_run=recorded_run,
                    node_id=_normalize_text_value(local_progress_payload.get("node_id")),
                ),
            }
        worker_started_payload = {
            "graph_run_id": graph_run_id,
            "graph_id": graph_id,
            "worker_session_id": worker_session_id,
            "worker_session_kind": worker_session.get("session_kind") or worker_session.get("kind") or "agent_run",
            "worker_title": worker_session.get("title") or worker_title,
            "packet_artifact_path": packet_bundle.get("packet_artifact_path"),
            "surface_artifact_path": packet_bundle.get("surface_artifact_path"),
            "turn_artifact_path": packet_bundle.get("turn_artifact_path"),
            "packet_root_path": packet_bundle.get("packet_root_path"),
            "packet_root_manifest_path": packet_bundle.get("packet_root_manifest_path"),
            "comparison_inputs_packaged": packet_bundle.get("comparison_inputs_packaged"),
            "comparison_evidence_artifact_path": packet_bundle.get("comparison_evidence_artifact_path"),
            "execution_mode": packet_bundle.get("execution_mode"),
            "final_remote_disposition_default": final_remote_disposition_default,
            "reply_poll_timeout_seconds": reply_poll_timeout_seconds,
            "turn_surface": "task_dag_compact_packet",
            "reply_excerpt": reply_excerpt,
            "turn_delivery_status": turn_delivery_status,
            "assistant_reply_sequence": outcome.get("assistant_reply_sequence"),
            "boundary_status": boundary_report.get("status"),
            "change_focus_policy": compact_packet_surface.get("change_focus_policy") or {},
            "next_focus_node_id": worker_metadata.get("next_focus_node_id"),
            "next_focus_task_id": worker_metadata.get("next_focus_task_id"),
            "next_focus_change_id": worker_metadata.get("next_focus_change_id"),
        }
        usage = outcome.get("usage") if isinstance(outcome.get("usage"), dict) else {}
        if usage:
            worker_started_payload["usage"] = usage
        if graph_run_session_id:
            if focus_metadata.get("node_id"):
                remote_append_session_event(
                    remote_row["url"],
                    graph_run_session_id,
                    "task_graph.node_local_progress",
                    {
                        "node_id": focus_metadata.get("node_id"),
                        "status": "running",
                        "task_id": focus_metadata.get("task_id"),
                        "change_id": focus_metadata.get("change_id"),
                        "worker_session_id": worker_session_id,
                        "summary": "Compact DAG worker session started local-first execution for the current focus node.",
                    },
                    repo_name=repo_name,
                )
            if local_progress_payload is not None:
                event_type = (
                    "task_graph.node_completed"
                    if str(local_progress_payload.get("status") or "").strip().lower() == "completed"
                    else "task_graph.node_local_progress"
                )
                remote_append_session_event(
                    remote_row["url"],
                    graph_run_session_id,
                    event_type,
                    {
                        "graph_run_id": graph_run_id,
                        "graph_id": graph_id,
                        **local_progress_payload,
                    },
                    repo_name=repo_name,
                )
            remote_append_session_event(
                remote_row["url"],
                graph_run_session_id,
                "task_graph.compact_worker_started",
                worker_started_payload,
                repo_name=repo_name,
            )
            remote_append_session_event(
                remote_row["url"],
                graph_run_session_id,
                "task_graph.compact_worker_boundary_report",
                {
                    "graph_run_id": graph_run_id,
                    "graph_id": graph_id,
                    "worker_session_id": worker_session_id,
                    **boundary_report,
                },
                repo_name=repo_name,
            )
        should_refresh_execute_run = bool(graph_run_session_id) and auto_continue_supported and comparison_evidence is None
        if not should_refresh_execute_run:
            break
        refreshed_readiness = _task_dag_readiness_payload(
            ctx,
            graph,
            _normalize_text_value(remote_row.get("name")) or _normalize_text_value(remote_row.get("remote_name")),
        )
        post_worker_run = _task_dag_refresh_execute_run(
            ctx=ctx,
            remote_row=remote_row,
            repo_name=repo_name,
            plan_id=plan_id,
            graph=graph,
            graph_path=graph_path,
            readiness=refreshed_readiness,
            session_id=graph_run_session_id,
            trigger="worker_session_completion",
            worker_session_id=worker_session_id,
        )
        if str((post_worker_run or {}).get("execution_state") or "").strip() == "completed":
            worker_session_close_out = remote_close_session(
                remote_row["url"],
                worker_session_id,
                status="completed",
                repo_name=repo_name,
            )
            graph_run_close_out = remote_close_session(
                remote_row["url"],
                graph_run_session_id,
                status="completed",
                repo_name=repo_name,
            )
            break
        latest_snapshot = (
            post_worker_run.get("latest_state_snapshot")
            if isinstance(post_worker_run.get("latest_state_snapshot"), dict)
            else {}
        )
        ready_node_ids = [str(value).strip() for value in latest_snapshot.get("ready_node_ids") or [] if str(value).strip()]
        if not ready_node_ids or str(latest_snapshot.get("execution_state") or "").strip() != "active":
            break
        next_surface = _task_dag_compact_worker_surface_from_snapshot(compact_packet_surface, latest_snapshot)
        next_surface, _ = _task_dag_seed_compact_worker_focus_lineage(
            ctx=ctx,
            remote_row=remote_row,
            repo_name=repo_name,
            plan_id=plan_id,
            graph=graph,
            graph_path=graph_path,
            recorded_run=recorded_run,
            compact_packet_surface=next_surface,
        )
        next_focus = _task_dag_compact_worker_focus_lineage(next_surface, recorded_run=recorded_run)
        next_focus_node_id = _normalize_text_value(next_focus.get("node_id"))
        current_focus_node_id = _normalize_text_value(worker_metadata.get("next_focus_node_id"))
        if next_focus_node_id is None or next_focus_node_id == current_focus_node_id:
            break
        next_iteration = _task_dag_prepare_auto_compact_worker_iteration(
            ctx=ctx,
            remote_row=remote_row,
            repo_name=repo_name,
            plan_id=plan_id,
            graph=graph,
            graph_path=graph_path,
            recorded_run=recorded_run,
            compact_packet_surface=next_surface,
            comparison_evidence=comparison_evidence,
            graph_run_session_id=graph_run_session_id,
            graph_run_id=graph_run_id,
        )
        continuation_count += 1
        continued_focus_node_ids.append(next_focus_node_id)
        compact_packet_surface = next_iteration["compact_packet_surface"]
        packet_bundle = next_iteration["packet_bundle"]
        final_remote_disposition_default = bool(next_iteration["final_remote_disposition_default"])
        worker_workspace_root = str(next_iteration["worker_workspace_root"])
        worker_repo_root = str(next_iteration["worker_repo_root"])
        reply_poll_timeout_seconds = float(next_iteration["reply_poll_timeout_seconds"])
        worker_metadata = dict(next_iteration["worker_metadata"])
        iteration = next_iteration
    else:
        raise ValueError("Compact DAG auto worker exceeded the guarded in-session continuation budget.")
    return {
        "graph_run_session_id": graph_run_session_id,
        "graph_run_id": graph_run_id,
        "packet_available": packet_bundle.get("packet_available"),
        "packet_artifact_path": packet_bundle.get("packet_artifact_path"),
        "surface_artifact_path": packet_bundle.get("surface_artifact_path"),
        "turn_artifact_path": packet_bundle.get("turn_artifact_path"),
        "packet_root_path": packet_bundle.get("packet_root_path"),
        "packet_root_manifest_path": packet_bundle.get("packet_root_manifest_path"),
        "comparison_inputs_packaged": packet_bundle.get("comparison_inputs_packaged"),
        "comparison_evidence_artifact_path": packet_bundle.get("comparison_evidence_artifact_path"),
        "packet_root_policy": packet_bundle.get("packet_root_policy") or {},
        "execution_mode": packet_bundle.get("execution_mode"),
        "final_remote_disposition_default": final_remote_disposition_default,
        "reply_poll_timeout_seconds": reply_poll_timeout_seconds,
        "worker_workspace_root": packet_bundle.get("worker_workspace_root"),
        "worker_repo_root": packet_bundle.get("worker_repo_root"),
        "worker_session_id": worker_session_id,
        "worker_session_kind": worker_session.get("session_kind") or worker_session.get("kind") or "agent_run",
        "worker_title": worker_session.get("title") or worker_title,
        "turn_surface": "task_dag_compact_packet",
        "turn_delivery_status": turn_delivery_status,
        "assistant_reply_sequence": outcome.get("assistant_reply_sequence"),
        "model": outcome.get("model"),
        "response_id": outcome.get("response_id"),
        "usage": outcome.get("usage") or {},
        "reply_text": outcome.get("reply_text"),
        "turn_analysis": outcome.get("turn_analysis") or {},
        "boundary_report": boundary_report,
        "local_progress_payload": local_progress_payload,
        "graph_run_close_out": graph_run_close_out,
        "worker_session_close_out": worker_session_close_out,
        "change_focus_policy": compact_packet_surface.get("change_focus_policy") or {},
        "next_focus_node_id": worker_metadata.get("next_focus_node_id"),
        "next_focus_task_id": worker_metadata.get("next_focus_task_id"),
        "next_focus_change_id": worker_metadata.get("next_focus_change_id"),
        "continuation_count": continuation_count,
        "continued_focus_node_ids": continued_focus_node_ids,
        "post_worker_run": post_worker_run,
    }
