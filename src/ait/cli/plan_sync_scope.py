from __future__ import annotations

import hashlib
import shutil
import tempfile
from contextlib import contextmanager
from pathlib import Path
from typing import Any, Iterable

import typer

from ..local_content import IGNORED_DIRS, ensure_blob_bytes, workspace_path_is_ignored
from ..plan_graph import load_task_graph
from ..remote_client import (
    create_plan as remote_create_plan,
    get_plan as remote_get_plan,
    list_plans as remote_list_plans,
    put_plan_revision_artifacts as remote_put_plan_revision_artifacts,
    revise_plan as remote_revise_plan,
    update_plan_status as remote_update_plan_status,
)
from ..repo_paths import RepoContext
from ..store import (
    close_local_plan,
    create_local_plan,
    get_local_plan,
    list_local_plans,
    restore_workspace_paths as local_restore_workspace_paths,
    revise_local_plan,
    workspace_status as local_workspace_status,
)
from .plan_sync_matching import (
    _artifact_candidates_open,
    _plan_matches_sync_artifact,
)
from .remote_repository_defaults import _remote_tuple, _sync_remote_repository_defaults
from .workflow_mode_config import _normalize_text_value

def _is_markdown_artifact_path(path_value: str) -> bool:
    return Path(str(path_value)).suffix.lower() == ".md"


def _is_forbidden_plan_sync_markdown_artifact_path(path_value: str | Path) -> bool:
    path = Path(str(path_value)).as_posix().strip("/")
    return path.lower() in {
        "docs/sprints/readme.md",
        "ait-dag.md",
    }


def _raise_forbidden_plan_sync_markdown_artifact(path_value: str | Path) -> None:
    normalized = Path(str(path_value)).as_posix().strip("/")
    if normalized.lower() == "ait-dag.md":
        raise typer.BadParameter(
            "ait-dag.md is reserved and cannot be used with `ait plan sync`. "
            "It is a compact-DAG runtime helper that should only be materialized inside packet "
            "`authoring_workspace_context`, not authored or synced as repo-root plan Markdown. "
            "Sync the real source sprint card such as docs/sprints/<card>.md instead."
        )
    raise typer.BadParameter(
        f"{normalized} is reserved and cannot be used with `ait plan sync`. "
        "Use a real sprint artifact path such as docs/sprints/<card>.md instead."
    )

def _resolve_repo_artifact_path(ctx: RepoContext, path_value: Path, *, allow_missing: bool = False) -> tuple[Path, str]:
    resolved_path = path_value.expanduser()
    if not resolved_path.is_absolute():
        resolved_path = (ctx.root / resolved_path).resolve(strict=False)
    else:
        resolved_path = resolved_path.resolve(strict=False)
    if not allow_missing and not resolved_path.exists():
        raise typer.BadParameter(f"Path does not exist: {path_value}")
    try:
        artifact_path = resolved_path.relative_to(ctx.root.resolve()).as_posix()
    except ValueError as exc:
        raise typer.BadParameter(f"Path must live inside the repository root: {path_value}") from exc
    if artifact_path == ".ait" or artifact_path.startswith(".ait/"):
        raise typer.BadParameter("Plan artifacts must be authored repository Markdown files, not runtime metadata under `.ait/`.")
    return resolved_path, artifact_path

def _resolve_plan_sync_target(
    ctx: RepoContext,
    target: Path,
    *,
    allow_missing: bool,
) -> dict[str, Any]:
    resolved_target, artifact_path = _resolve_repo_artifact_path(ctx, target, allow_missing=allow_missing)
    if resolved_target.exists() and resolved_target.is_dir():
        resolved_root = ctx.root.resolve()
        files = sorted(
            path
            for path in resolved_target.rglob("*.md")
            if path.is_file()
            and not any(part in IGNORED_DIRS for part in path.relative_to(resolved_root).parts)
            and not workspace_path_is_ignored(ctx.root, path.relative_to(resolved_root))
            and not _is_forbidden_plan_sync_markdown_artifact_path(path.relative_to(resolved_root).as_posix())
        )
        return {
            "scope": "directory",
            "target_path": artifact_path.rstrip("/"),
            "resolved_target": resolved_target,
            "files": files,
        }
    if resolved_target.exists() and resolved_target.is_file():
        if resolved_target.suffix.lower() != ".md":
            raise typer.BadParameter(f"Plan sync target must be a Markdown file or directory: {target}")
        if _is_forbidden_plan_sync_markdown_artifact_path(artifact_path):
            _raise_forbidden_plan_sync_markdown_artifact(artifact_path)
        return {
            "scope": "file",
            "target_path": artifact_path,
            "resolved_target": resolved_target,
            "files": [resolved_target],
        }
    if allow_missing and artifact_path.endswith(".md"):
        if _is_forbidden_plan_sync_markdown_artifact_path(artifact_path):
            _raise_forbidden_plan_sync_markdown_artifact(artifact_path)
        return {
            "scope": "file",
            "target_path": artifact_path,
            "resolved_target": resolved_target,
            "files": [],
        }
    if allow_missing:
        raise typer.BadParameter(
            f"Missing sync target {target}. Use an existing directory, or point plan sync at one specific Markdown path when publishing or pruning a deletion."
        )
    raise typer.BadParameter(f"Plan sync target does not exist: {target}")

def _plan_sync_task_graph_metadata(ctx: RepoContext, artifact_path: str, graph: dict[str, Any]) -> dict[str, Any]:
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    dispatch_artifacts = graph.get("dispatch_artifacts") if isinstance(graph.get("dispatch_artifacts"), dict) else {}
    execution_policy = graph.get("execution_policy") if isinstance(graph.get("execution_policy"), dict) else {}
    return {
        "artifact_kind": "task_graph_json",
        "graph_id": graph.get("graph_id"),
        "source_plan": source_plan,
        "dispatch_artifacts": dispatch_artifacts,
        "execution_policy": {
            "validate_source_plan_revision": bool(execution_policy.get("validate_source_plan_revision")),
            "mode": execution_policy.get("mode"),
        },
        "task_graph_json": artifact_path,
        "loaded_from": artifact_path,
    }

def _resolve_plan_sync_task_graph_artifact(ctx: RepoContext, path_value: Path) -> dict[str, Any]:
    resolved_path, artifact_path = _resolve_repo_artifact_path(ctx, path_value)
    if not resolved_path.is_file():
        raise typer.BadParameter(f"Plan sync artifact must be a file: {path_value}")
    if resolved_path.suffix.lower() != ".json" or not resolved_path.name.endswith(".task_graph.json"):
        raise typer.BadParameter(f"Plan sync paired artifacts currently support stable *.task_graph.json files only: {path_value}")
    graph = load_task_graph(resolved_path)
    body = resolved_path.read_text(encoding="utf-8")
    data = body.encode("utf-8")
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    source_artifact_path = _normalize_text_value(source_plan.get("artifact_path"))
    if source_artifact_path is None:
        raise typer.BadParameter(f"Task graph artifact {artifact_path} is missing source_plan.artifact_path.")
    return {
        "artifact_path": artifact_path,
        "source_artifact_path": source_artifact_path,
        "role": "task_graph_json",
        "media_type": "application/vnd.ait.task-graph+json",
        "encoding": "utf-8",
        "body": body,
        "byte_count": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
        "graph": graph,
        "metadata": _plan_sync_task_graph_metadata(ctx, artifact_path, graph),
    }

def _plan_sync_auto_task_graph_candidates(ctx: RepoContext, sync_target: dict[str, Any]) -> list[Path]:
    resolved_target = sync_target.get("resolved_target")
    if not isinstance(resolved_target, Path) or not resolved_target.exists():
        return []
    resolved_root = ctx.root.resolve()
    if resolved_target.is_file():
        candidate = resolved_target.with_name(f"{resolved_target.stem}.task_graph.json")
        return [candidate] if candidate.exists() and candidate.is_file() else []
    candidates: list[Path] = []
    for path in resolved_target.rglob("*.task_graph.json"):
        if not path.is_file():
            continue
        relative = path.relative_to(resolved_root)
        if any(part in IGNORED_DIRS for part in relative.parts):
            continue
        if workspace_path_is_ignored(ctx.root, relative):
            continue
        candidates.append(path)
    return sorted(candidates)

def _resolve_plan_sync_paired_artifacts(
    ctx: RepoContext,
    *,
    sync_target: dict[str, Any],
    markdown_artifacts: list[dict[str, Any]],
    publish_remote: bool,
) -> dict[str, list[dict[str, Any]]]:
    if not publish_remote:
        return {}
    markdown_paths = {str(artifact["artifact_path"]) for artifact in markdown_artifacts}
    if not markdown_paths:
        return {}
    artifact_paths: dict[str, Path] = {
        str(path.resolve(strict=False)): path for path in _plan_sync_auto_task_graph_candidates(ctx, sync_target)
    }
    grouped: dict[str, list[dict[str, Any]]] = {}
    for path in artifact_paths.values():
        artifact = _resolve_plan_sync_task_graph_artifact(ctx, path)
        source_artifact_path = str(artifact["source_artifact_path"])
        if source_artifact_path not in markdown_paths:
            raise ValueError(
                f"Task graph artifact {artifact['artifact_path']} points at {source_artifact_path}, "
                "which is not part of this plan sync target."
            )
        grouped.setdefault(source_artifact_path, []).append(artifact)
    for rows in grouped.values():
        rows.sort(key=lambda row: str(row.get("artifact_path") or ""))
    return grouped

def _remote_revision_id_for_synced_plan(ctx: RepoContext, plan_id: str, remote_name: str | None) -> str:
    local_plan = get_local_plan(ctx, plan_id)
    remote_revision_id = _normalize_text_value(local_plan.get("published_head_revision_id"))
    if remote_revision_id is not None:
        return remote_revision_id
    remote_row, _ = _remote_tuple(ctx, remote_name)
    remote_plan = remote_get_plan(remote_row["url"], plan_id)
    remote_head = remote_plan.get("head_revision") if isinstance(remote_plan.get("head_revision"), dict) else {}
    remote_revision_id = _normalize_text_value(remote_head.get("plan_revision_id") or remote_plan.get("head_revision_id"))
    if remote_revision_id is None:
        raise ValueError(f"Remote plan {plan_id} has no head revision for paired artifact upload.")
    return remote_revision_id

def _validate_plan_sync_task_graph_artifact_for_revision(
    artifact: dict[str, Any],
    *,
    plan_id: str,
    plan_revision_id: str,
    markdown_artifact_path: str,
) -> None:
    graph = artifact.get("graph") if isinstance(artifact.get("graph"), dict) else {}
    source_plan = graph.get("source_plan") if isinstance(graph.get("source_plan"), dict) else {}
    graph_plan_id = _normalize_text_value(source_plan.get("plan_id"))
    graph_revision_id = _normalize_text_value(source_plan.get("plan_revision_id"))
    graph_artifact_path = _normalize_text_value(source_plan.get("artifact_path"))
    if graph_plan_id != plan_id:
        raise ValueError(f"Task graph artifact {artifact['artifact_path']} belongs to plan {graph_plan_id!r}, not {plan_id!r}.")
    if graph_artifact_path != markdown_artifact_path:
        raise ValueError(
            f"Task graph artifact {artifact['artifact_path']} points at {graph_artifact_path!r}, "
            f"not synced Markdown artifact {markdown_artifact_path!r}."
        )
    execution_policy = graph.get("execution_policy") if isinstance(graph.get("execution_policy"), dict) else {}
    if bool(execution_policy.get("validate_source_plan_revision")) and graph_revision_id != plan_revision_id:
        raise ValueError(
            f"Task graph artifact {artifact['artifact_path']} is stale for plan {plan_id}: "
            f"source_plan.plan_revision_id is {graph_revision_id!r}, current remote head is {plan_revision_id!r}."
        )

def _publish_plan_sync_paired_artifacts(
    ctx: RepoContext,
    *,
    results: list[dict[str, Any]],
    paired_artifacts_by_markdown_path: dict[str, list[dict[str, Any]]],
    remote_name: str | None,
) -> list[dict[str, Any]]:
    if not paired_artifacts_by_markdown_path:
        return []
    if remote_name is None:
        raise ValueError("Paired plan sync artifacts require a remote publish target.")
    remote_row, _ = _remote_tuple(ctx, remote_name)
    uploads: list[dict[str, Any]] = []
    for row in results:
        markdown_artifact_path = _normalize_text_value(row.get("artifact_path"))
        plan_id = _normalize_text_value(row.get("plan_id"))
        if markdown_artifact_path is None or plan_id is None:
            continue
        artifacts = paired_artifacts_by_markdown_path.get(markdown_artifact_path) or []
        if not artifacts:
            continue
        plan_revision_id = _remote_revision_id_for_synced_plan(ctx, plan_id, remote_name)
        request_artifacts: list[dict[str, Any]] = []
        for artifact in artifacts:
            _validate_plan_sync_task_graph_artifact_for_revision(
                artifact,
                plan_id=plan_id,
                plan_revision_id=plan_revision_id,
                markdown_artifact_path=markdown_artifact_path,
            )
            request_artifacts.append(
                {
                    "artifact_path": artifact["artifact_path"],
                    "role": artifact["role"],
                    "media_type": artifact["media_type"],
                    "encoding": artifact["encoding"],
                    "body": artifact["body"],
                    "metadata": artifact["metadata"],
                }
            )
        uploaded = remote_put_plan_revision_artifacts(remote_row["url"], plan_id, plan_revision_id, request_artifacts)
        for uploaded_artifact in uploaded.get("artifacts") or []:
            uploads.append(
                {
                    "plan_id": plan_id,
                    "plan_revision_id": plan_revision_id,
                    "source_artifact_path": markdown_artifact_path,
                    "artifact_path": uploaded_artifact.get("artifact_path"),
                    "role": uploaded_artifact.get("role"),
                    "blob_id": uploaded_artifact.get("blob_id"),
                    "sha256": uploaded_artifact.get("sha256"),
                    "byte_count": uploaded_artifact.get("byte_count"),
                }
            )
    return uploads

def _artifact_path_in_sync_scope(ctx: RepoContext, artifact_path: str, *, scope: str, target_path: str) -> bool:
    if scope == "file":
        return artifact_path == target_path
    normalized_target = target_path.rstrip("/")
    if normalized_target in {"", "."}:
        rel_path = Path(artifact_path)
        if any(part in IGNORED_DIRS for part in rel_path.parts):
            return False
        if workspace_path_is_ignored(ctx.root, rel_path):
            return False
        return True
    return artifact_path == normalized_target or artifact_path.startswith(f"{normalized_target}/")

def _load_plan_sync_existing_plans(
    ctx: RepoContext,
    *,
    local: bool,
    remote_name: str | None,
) -> tuple[list[dict[str, Any]], dict[str, Any] | None, str | None]:
    if local:
        rows = list_local_plans(ctx)
        return [get_local_plan(ctx, str(row["plan_id"])) for row in rows], None, None
    remote_row, repo_name = _sync_remote_repository_defaults(ctx, remote_name)
    rows = remote_list_plans(remote_row["url"], repo_name)
    return [remote_get_plan(remote_row["url"], str(row["plan_id"])) for row in rows], remote_row, repo_name

def _sync_single_plan_artifact(
    ctx: RepoContext,
    artifact: dict[str, Any],
    *,
    local: bool,
    remote_row: dict[str, Any] | None,
    repo_name: str | None,
    existing_plan: dict[str, Any] | None,
    continuity_match: dict[str, Any] | None = None,
) -> dict[str, Any]:
    local_artifact_blob_id = artifact.get("artifact_blob_id")
    if local:
        artifact_body = artifact.get("artifact_body")
        if isinstance(artifact_body, str):
            local_artifact_blob_id = ensure_blob_bytes(
                ctx,
                artifact_body.encode("utf-8"),
                path_hint=str(artifact.get("artifact_path") or ""),
            )
    if existing_plan is None:
        if local:
            data = create_local_plan(
                ctx,
                artifact["artifact_heading"],
                artifact["artifact_path"],
                artifact["artifact_selector"],
                artifact["artifact_heading"],
                artifact["items"],
                artifact_blob_id=local_artifact_blob_id,
            )
        else:
            assert remote_row is not None
            assert repo_name is not None
            data = remote_create_plan(
                remote_row["url"],
                repo_name,
                artifact["artifact_heading"],
                artifact["artifact_path"],
                artifact["artifact_selector"],
                artifact["artifact_heading"],
                artifact["items"],
                artifact_body=artifact.get("artifact_body"),
            )
        return {
            "action": "created",
            "artifact_path": artifact["artifact_path"],
            "artifact_selector": artifact.get("artifact_selector"),
            "plan_id": data.get("plan_id"),
            "plan_revision_id": ((data.get("head_revision") or {}).get("plan_revision_id") or data.get("head_revision_id")),
            "status": data.get("status"),
        }

    if _plan_matches_sync_artifact(existing_plan, artifact):
        return {
            "action": "unchanged",
            "artifact_path": artifact["artifact_path"],
            "artifact_selector": artifact.get("artifact_selector"),
            "plan_id": existing_plan.get("plan_id"),
            "plan_revision_id": ((existing_plan.get("head_revision") or {}).get("plan_revision_id") or existing_plan.get("head_revision_id")),
            "status": existing_plan.get("status"),
        }

    if local:
        data = revise_local_plan(
            ctx,
            str(existing_plan["plan_id"]),
            artifact["artifact_path"],
            artifact["artifact_selector"],
            artifact["artifact_heading"],
            artifact["items"],
            artifact_blob_id=local_artifact_blob_id,
            title=artifact["artifact_heading"],
        )
    else:
        assert remote_row is not None
        head_revision = existing_plan.get("head_revision") or {}
        expected_head_revision_id = head_revision.get("plan_revision_id") or existing_plan.get("head_revision_id")
        data = remote_revise_plan(
            remote_row["url"],
            str(existing_plan["plan_id"]),
            artifact["artifact_path"],
            artifact["artifact_selector"],
            artifact["artifact_heading"],
            artifact["items"],
            title=artifact["artifact_heading"],
            artifact_body=artifact.get("artifact_body"),
            expected_head_revision_id=expected_head_revision_id,
        )
    return {
        "action": "updated",
        "artifact_path": artifact["artifact_path"],
        "artifact_selector": artifact.get("artifact_selector"),
        "plan_id": data.get("plan_id"),
        "plan_revision_id": ((data.get("head_revision") or {}).get("plan_revision_id") or data.get("head_revision_id")),
        "status": data.get("status"),
        **({"continuity_match": continuity_match} if continuity_match is not None else {}),
    }

def _prune_missing_plan_artifacts(
    ctx: RepoContext,
    *,
    local: bool,
    remote_row: dict[str, Any] | None,
    sync_target: dict[str, Any],
    indexed_plans: dict[str, list[dict[str, Any]]],
    synced_artifact_paths: set[str],
) -> list[dict[str, Any]]:
    results: list[dict[str, Any]] = []
    for artifact_path, candidates in sorted(indexed_plans.items()):
        if not _artifact_path_in_sync_scope(
            ctx,
            artifact_path,
            scope=str(sync_target["scope"]),
            target_path=str(sync_target["target_path"]),
        ):
            continue
        if artifact_path in synced_artifact_paths:
            continue
        absolute_path = ctx.root / artifact_path
        if absolute_path.exists():
            continue
        open_candidates = _artifact_candidates_open(candidates)
        if not open_candidates:
            continue
        for existing_plan in open_candidates:
            if local:
                data = close_local_plan(ctx, str(existing_plan["plan_id"]), "archived")
            else:
                assert remote_row is not None
                data = remote_update_plan_status(remote_row["url"], str(existing_plan["plan_id"]), "archived")
            results.append(
                {
                    "action": "pruned",
                    "artifact_path": artifact_path,
                    "artifact_selector": _normalize_text_value((existing_plan.get("head_revision") or {}).get("artifact_selector")),
                    "plan_id": data.get("plan_id"),
                    "plan_revision_id": ((data.get("head_revision") or {}).get("plan_revision_id") or data.get("head_revision_id")),
                    "status": data.get("status"),
                }
            )
    return results

def _tracked_missing_markdown_artifact_paths(
    ctx: RepoContext,
    *,
    sync_target: dict[str, Any],
    artifact_paths: Iterable[str],
    synced_artifact_paths: set[str],
) -> set[str]:
    deleted: set[str] = set()
    for artifact_path in artifact_paths:
        normalized_path = str(artifact_path or "").strip()
        if not normalized_path or normalized_path in synced_artifact_paths:
            continue
        if not _is_markdown_artifact_path(normalized_path):
            continue
        if not _artifact_path_in_sync_scope(
            ctx,
            normalized_path,
            scope=str(sync_target["scope"]),
            target_path=str(sync_target["target_path"]),
        ):
            continue
        absolute_path = ctx.root / normalized_path
        if absolute_path.exists():
            continue
        deleted.add(normalized_path)
    return deleted

def _prune_empty_workspace_parent_dirs(root: Path, path: Path) -> None:
    root_resolved = root.resolve(strict=False)
    current = path.parent.resolve(strict=False)
    while current != root_resolved:
        try:
            current.rmdir()
        except OSError:
            break
        current = current.parent.resolve(strict=False)

@contextmanager
def _preserve_workspace_paths_for_plan_sync(
    ctx: RepoContext,
    *,
    paths: set[str],
    lock_reason: str,
) -> dict[str, Any]:
    from .app import _run_locked_workspace_command

    preserved_paths = sorted(str(path) for path in paths if str(path))
    payload: dict[str, Any] = {
        "paths": preserved_paths,
        "modified_paths": [],
        "missing_paths": [],
        "untracked_paths": [],
        "tracked_restore_paths": [],
    }
    if not preserved_paths:
        yield payload
        return

    status = local_workspace_status(ctx)
    modified_paths = sorted(set(str(path) for path in status.get("modified_paths") or []) & set(preserved_paths))
    missing_paths = sorted(set(str(path) for path in status.get("missing_paths") or []) & set(preserved_paths))
    untracked_paths = sorted(set(str(path) for path in status.get("untracked_paths") or []) & set(preserved_paths))
    tracked_restore_paths = sorted(set(modified_paths) | set(missing_paths))
    payload.update(
        {
            "modified_paths": modified_paths,
            "missing_paths": missing_paths,
            "untracked_paths": untracked_paths,
            "tracked_restore_paths": tracked_restore_paths,
        }
    )
    if not tracked_restore_paths and not untracked_paths:
        yield payload
        return

    backup_root = Path(tempfile.mkdtemp(prefix="ait-plan-sync-preserve-"))
    payload["backup_root"] = str(backup_root)
    try:
        for rel_path in modified_paths:
            source_path = (ctx.root / rel_path).resolve(strict=False)
            if not source_path.exists():
                raise ValueError(f"Cannot preserve modified workspace path {rel_path}: path does not exist")
            if source_path.is_dir():
                raise IsADirectoryError(f"Cannot preserve modified workspace path {rel_path}: path is a directory")
            backup_path = backup_root / rel_path
            backup_path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source_path, backup_path)
        for rel_path in untracked_paths:
            source_path = (ctx.root / rel_path).resolve(strict=False)
            if not source_path.exists():
                continue
            backup_path = backup_root / rel_path
            backup_path.parent.mkdir(parents=True, exist_ok=True)
            shutil.move(str(source_path), str(backup_path))
            _prune_empty_workspace_parent_dirs(ctx.root, source_path)
        if tracked_restore_paths:
            _run_locked_workspace_command(
                ctx,
                lock_reason,
                lambda: local_restore_workspace_paths(ctx, tracked_restore_paths, force=True),
            )
        yield payload
    finally:
        restore_errors: list[str] = []
        for rel_path in modified_paths:
            backup_path = backup_root / rel_path
            if not backup_path.exists():
                restore_errors.append(f"{rel_path}: modified backup missing")
                continue
            target_path = (ctx.root / rel_path).resolve(strict=False)
            try:
                target_path.parent.mkdir(parents=True, exist_ok=True)
                if target_path.exists() and target_path.is_dir():
                    raise IsADirectoryError(f"Cannot restore file over directory: {rel_path}")
                shutil.copy2(backup_path, target_path)
            except OSError as exc:
                restore_errors.append(f"{rel_path}: {exc}")
        for rel_path in missing_paths:
            target_path = (ctx.root / rel_path).resolve(strict=False)
            try:
                if target_path.exists():
                    if target_path.is_dir():
                        raise IsADirectoryError(f"Cannot restore missing-file deletion over directory: {rel_path}")
                    target_path.unlink()
                    _prune_empty_workspace_parent_dirs(ctx.root, target_path)
            except OSError as exc:
                restore_errors.append(f"{rel_path}: {exc}")
        for rel_path in untracked_paths:
            backup_path = backup_root / rel_path
            if not backup_path.exists():
                continue
            target_path = (ctx.root / rel_path).resolve(strict=False)
            try:
                target_path.parent.mkdir(parents=True, exist_ok=True)
                if target_path.exists():
                    if target_path.is_dir():
                        raise IsADirectoryError(f"Cannot restore untracked path over directory: {rel_path}")
                    raise FileExistsError(f"Cannot restore untracked path because {rel_path} already exists")
                shutil.move(str(backup_path), str(target_path))
            except OSError as exc:
                restore_errors.append(f"{rel_path}: {exc}")
        shutil.rmtree(backup_root, ignore_errors=True)
        if restore_errors:
            joined = "; ".join(restore_errors[:5])
            if len(restore_errors) > 5:
                joined += "; ..."
            raise ValueError(f"Failed to restore preserved workspace paths after plan sync: {joined}")
