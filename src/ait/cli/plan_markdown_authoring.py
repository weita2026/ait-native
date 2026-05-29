from __future__ import annotations

import re
import shlex
from datetime import datetime
from pathlib import Path
from typing import Any, Mapping

import typer

from ait_protocol.common import extract_plan_section, list_plan_section_refs

from ..local_content_projection import _path_is_projected_out_for_workspace
from ..remote_client import RemoteError
from ..repo_paths import RepoContext
from ..store import (
    get_snapshot,
    get_worktree as local_get_worktree,
    iter_workspace_files,
    load_config,
    workspace_status as local_workspace_status,
)
from .plan_sync_matching import (
    _artifact_blob_id,
    _artifact_candidates_open,
    _index_plans_by_artifact_path,
    _plan_matches_sync_artifact,
)
from .plan_sync_scope import (
    _is_markdown_artifact_path,
    _load_plan_sync_existing_plans,
    _resolve_repo_artifact_path,
)
from .workflow_mode_config import _effective_workflow_mode, _normalize_text_value


def _is_plan_sync_only_sprint_task_graph_path(path_value: str | Path) -> bool:
    path = Path(str(path_value).replace("\\", "/")).as_posix().strip("/")
    return path.startswith("docs/sprints/") and path.endswith(".task_graph.json")


def _markdown_changed_paths(status: dict[str, Any]) -> list[str]:
    return sorted(
        str(path)
        for path in status.get("changed_paths") or []
        if _is_markdown_artifact_path(str(path))
    )


def _timestamp_from_iso(value: Any) -> float | None:
    text = _normalize_text_value(str(value)) if value is not None else None
    if text is None:
        return None
    try:
        return datetime.fromisoformat(text.replace("Z", "+00:00")).timestamp()
    except ValueError:
        return None


def _snapshot_workspace_marker_timestamp(ctx: RepoContext, snapshot: Mapping[str, Any]) -> float | None:
    manifest_path = _normalize_text_value(snapshot.get("manifest_path"))
    if manifest_path is not None:
        manifest_storage_path = manifest_path.split("#", 1)[0]
        try:
            return (ctx.root / manifest_storage_path).stat().st_mtime
        except OSError:
            pass
    return _timestamp_from_iso(snapshot.get("created_at"))


def _root_markdown_paths_modified_after_baseline(ctx: RepoContext, status: dict[str, Any]) -> list[str]:
    if ctx.is_worktree:
        return []
    baseline_snapshot_id = _normalize_text_value(status.get("baseline_snapshot_id"))
    if baseline_snapshot_id is None:
        return []
    try:
        baseline = get_snapshot(ctx, baseline_snapshot_id)
    except KeyError:
        return []
    baseline_timestamp = _snapshot_workspace_marker_timestamp(ctx, baseline)
    if baseline_timestamp is None:
        return []
    paths: list[str] = []
    for path in iter_workspace_files(ctx.root):
        rel = path.relative_to(ctx.root).as_posix()
        if not _is_markdown_artifact_path(rel):
            continue
        try:
            modified_at = path.stat().st_mtime
        except OSError:
            continue
        if modified_at > baseline_timestamp:
            paths.append(rel)
    return sorted(paths)


def _workspace_dispatch_authored_markdown_paths(ctx: RepoContext, status: dict[str, Any]) -> list[str]:
    paths = set(_markdown_changed_paths(status))
    if ctx.is_worktree:
        paths.update(_execution_worktree_planning_only_paths(ctx, status))
    else:
        paths.update(_root_markdown_paths_modified_after_baseline(ctx, status))
    return sorted(paths)


def _markdown_changed_missing_paths(status: dict[str, Any]) -> set[str]:
    return {
        str(path)
        for path in status.get("missing_paths") or []
        if _is_markdown_artifact_path(str(path))
    }


def _current_worktree_name(ctx: RepoContext) -> str:
    if not ctx.is_worktree:
        return "current worktree"
    try:
        worktree = local_get_worktree(ctx)
    except KeyError:
        return "current worktree"
    return str(worktree.get("name") or "current worktree")


def _execution_worktree_authored_markdown_paths(ctx: RepoContext) -> list[str]:
    if not ctx.is_worktree:
        return []
    paths: list[str] = []
    for path in iter_workspace_files(ctx.root):
        rel = path.relative_to(ctx.root).as_posix()
        if _is_markdown_artifact_path(rel) and _path_is_projected_out_for_workspace(ctx, rel):
            paths.append(rel)
    return sorted(paths)


def _execution_worktree_dirty_sprint_task_graph_paths(status: dict[str, Any]) -> list[str]:
    return sorted(
        str(path)
        for path in status.get("changed_paths") or []
        if _is_plan_sync_only_sprint_task_graph_path(str(path))
    )


def _execution_worktree_planning_only_paths(ctx: RepoContext, status: dict[str, Any]) -> list[str]:
    paths = set(_execution_worktree_authored_markdown_paths(ctx))
    paths.update(_execution_worktree_dirty_sprint_task_graph_paths(status))
    return sorted(paths)


def _worktree_docs_sprints_plan_sync_hint(targets: list[str]) -> str:
    normalized_targets = [Path(str(target)).as_posix().strip("/") for target in targets if str(target).strip()]
    if not any(target == "docs/sprints" or target.startswith("docs/sprints/") for target in normalized_targets):
        return ""
    return (
        " `docs/sprints/` copies inside execution worktrees or compact-DAG "
        "`authoring_workspace_context` are read-only planning/runtime context; "
        "plan-sync the repo-root source Markdown instead."
    )


def _guard_execution_worktree_plan_sync(ctx: RepoContext, *, target: Path | str) -> None:
    if not ctx.is_worktree:
        return
    normalized_target = Path(str(target)).as_posix().strip() or "."
    worktree_name = _current_worktree_name(ctx)
    docs_sprints_hint = _worktree_docs_sprints_plan_sync_hint([normalized_target])
    raise typer.BadParameter(
        "Execution worktrees cannot materialize, author, or plan-sync Markdown or planning-only sprint artifacts. "
        f"`ait plan sync {normalized_target}` must run from the repo root planning surface, not worktree "
        f"`{worktree_name}`.{docs_sprints_hint} Continue from the repo root `{ctx.repo_root}`."
    )


def _markdown_task_dispatch_sync_command(status: dict[str, Any], markdown_paths: list[str]) -> str:
    changed_paths = sorted(str(path) for path in markdown_paths if str(path))
    missing_paths = _markdown_changed_missing_paths(status) & set(changed_paths)
    if len(changed_paths) == 1:
        only_path = changed_paths[0]
        target = only_path if _is_markdown_artifact_path(only_path) else str(Path(only_path).parent or ".")
    else:
        parents = {str(Path(path).parent) for path in changed_paths}
        target = parents.pop() if len(parents) == 1 else "."
    command = f"ait plan sync {shlex.quote(target)}"
    if missing_paths:
        command += " --prune"
    return command


def _markdown_artifact_matches_tracked_plan(ctx: RepoContext, artifact_path: str, plan: dict[str, Any]) -> bool:
    absolute_path = (ctx.root / artifact_path).resolve(strict=False)
    if not absolute_path.exists() or not absolute_path.is_file():
        return False
    head_revision = plan.get("head_revision") if isinstance(plan.get("head_revision"), dict) else {}
    selector = _normalize_text_value(head_revision.get("artifact_selector"))
    try:
        artifact = _resolve_plan_artifact_input(
            ctx,
            absolute_path,
            selector,
            allow_generic_markdown=True,
        )
    except typer.BadParameter:
        return False
    return _plan_matches_sync_artifact(plan, artifact, require_title_match=False)


def _markdown_artifact_path_reconciled(ctx: RepoContext, artifact_path: str, *, indexed_plans: dict[str, list[dict[str, Any]]]) -> bool:
    candidates = indexed_plans.get(artifact_path, [])
    open_candidates = _artifact_candidates_open(candidates)
    absolute_path = (ctx.root / artifact_path).resolve(strict=False)
    if not absolute_path.exists():
        return bool(candidates) and not open_candidates
    if not absolute_path.is_file():
        return False
    if len(open_candidates) != 1:
        return False
    return _markdown_artifact_matches_tracked_plan(ctx, artifact_path, open_candidates[0])


def _unreconciled_markdown_paths(
    ctx: RepoContext,
    markdown_paths: list[str],
) -> tuple[list[str], list[str]]:
    local_plans, _, _ = _load_plan_sync_existing_plans(ctx, local=True, remote_name=None)
    local_indexed = _index_plans_by_artifact_path(local_plans)
    remaining: list[str] = []
    reconciled: list[str] = []
    for path in markdown_paths:
        if _markdown_artifact_path_reconciled(ctx, path, indexed_plans=local_indexed):
            reconciled.append(path)
        else:
            remaining.append(path)
    if not remaining:
        return [], reconciled

    try:
        remote_plans, _, _ = _load_plan_sync_existing_plans(ctx, local=False, remote_name=None)
    except (KeyError, RemoteError, ValueError):
        return remaining, reconciled
    remote_indexed = _index_plans_by_artifact_path(remote_plans)
    unresolved: list[str] = []
    for path in remaining:
        if _markdown_artifact_path_reconciled(ctx, path, indexed_plans=remote_indexed):
            reconciled.append(path)
        else:
            unresolved.append(path)
    return unresolved, reconciled


def _guard_markdown_task_dispatch(
    ctx: RepoContext,
    *,
    plan_id: str | None,
    plan_revision_id: str | None,
    plan_item_ref: str | None,
    command_name: str | None = None,
) -> None:
    from .app import _task_auto_worktree_bootstrap_allows_dirty_source

    status = local_workspace_status(ctx)
    if status.get("baseline_snapshot_id") is None:
        return
    if _task_auto_worktree_bootstrap_allows_dirty_source(ctx, command_name):
        return
    markdown_paths = _workspace_dispatch_authored_markdown_paths(ctx, status)
    if not markdown_paths:
        return
    unreconciled_paths, reconciled_paths = _unreconciled_markdown_paths(ctx, markdown_paths)
    if not unreconciled_paths:
        return
    command = _markdown_task_dispatch_sync_command(status, unreconciled_paths)
    sample = ", ".join(unreconciled_paths[:5])
    if len(unreconciled_paths) > 5:
        sample += f", ... (+{len(unreconciled_paths) - 5} more)"
    path_label = "Markdown paths" if all(_is_markdown_artifact_path(path) for path in unreconciled_paths) else "Planning-only paths"
    non_markdown_paths = sorted(
        str(path)
        for path in status.get("changed_paths") or []
        if not _is_markdown_artifact_path(str(path)) and not _is_plan_sync_only_sprint_task_graph_path(str(path))
    )
    extra = ""
    if non_markdown_paths:
        other_sample = ", ".join(non_markdown_paths[:5])
        if len(non_markdown_paths) > 5:
            other_sample += f", ... (+{len(non_markdown_paths) - 5} more)"
        extra = f" Other dirty paths remain after the Markdown sync queue: {other_sample}."
    reconciled_extra = ""
    if reconciled_paths:
        synced_sample = ", ".join(reconciled_paths[:3])
        if len(reconciled_paths) > 3:
            synced_sample += f", ... (+{len(reconciled_paths) - 3} more)"
        reconciled_extra = f" Already reconciled Markdown paths can remain dirty relative to the current line head: {synced_sample}."
    task_graph_hint = ""
    if any(_is_plan_sync_only_sprint_task_graph_path(path) for path in unreconciled_paths):
        task_graph_hint = (
            " Paired `docs/sprints/*.task_graph.json` artifacts must stay on the repo-root `ait plan sync ... --remote <name>` path,"
            " not inside task/change/snapshot/land authoring."
        )
    workflow_mode = str((_effective_workflow_mode(ctx) or {}).get("value") or "")
    default_remote = _normalize_text_value(load_config(ctx).get("default_remote"))
    scope_hint = " After that sync, task/change commands return to the repository's normal default scope."
    if workflow_mode == "solo_remote":
        if default_remote is not None:
            scope_hint = (
                " After that sync, new task/change commands usually follow the repository's "
                "remote-backed default without another `--remote`."
            )
        else:
            scope_hint = (
                " After that sync, new task/change commands follow the repository's remote-backed "
                "default; if no default remote is configured yet, pass `--remote <name>` or set one first."
            )
    raise ValueError(
        "Refusing to dispatch task/change workflow while authored Markdown drift is present or while planning-only sprint artifacts are dirty. "
        f"Reconcile it first with `{command}`"
        " and add `--remote <name>` only when the Markdown update must reach shared plan state."
        f"{scope_hint} "
        f"{path_label}: {sample}.{task_graph_hint}{reconciled_extra}{extra}"
    )


def _guard_execution_worktree_snapshot_markdown(ctx: RepoContext, *, command_name: str = "snapshot create") -> None:
    if not ctx.is_worktree:
        return
    status = local_workspace_status(ctx)
    planning_only_paths = _execution_worktree_planning_only_paths(ctx, status)
    if not planning_only_paths:
        return
    sample = ", ".join(planning_only_paths[:5])
    if len(planning_only_paths) > 5:
        sample += f", ... (+{len(planning_only_paths) - 5} more)"
    worktree_name = _current_worktree_name(ctx)
    docs_sprints_hint = _worktree_docs_sprints_plan_sync_hint(planning_only_paths)
    raise ValueError(
        "Execution worktrees cannot materialize, author, or snapshot planning-only sprint artifacts. "
        f"Worktree `{worktree_name}` must hand sprint Markdown / task-graph authoring back to the repo root planning surface "
        f"`{ctx.repo_root}` before `ait {command_name}`. Reconcile the authored Markdown from the repo root with "
        f"`ait plan sync <markdown-path-or-dir>` and let paired `docs/sprints/*.task_graph.json` artifacts ride through that same repo-root sync path before retrying.{docs_sprints_hint} Planning-only paths: {sample}."
    )


_MARKDOWN_HEADING_RE = re.compile(r"^(#{1,6})\s+(.+?)\s*$")


def _default_markdown_artifact_heading(markdown: str, artifact_path: str) -> str:
    for raw_line in markdown.splitlines():
        heading_match = _MARKDOWN_HEADING_RE.match(raw_line.strip())
        if heading_match is not None:
            return heading_match.group(2).strip()
    stem = Path(artifact_path).stem.replace("_", " ").replace("-", " ").strip()
    return stem or artifact_path


def _resolve_plan_artifact_input(
    ctx: RepoContext,
    body_file: Path | None,
    plan_ref: str | None,
    *,
    allow_generic_markdown: bool = False,
) -> dict[str, Any]:
    if body_file is None:
        raise typer.BadParameter("`ait plan` authoring requires `--file`.")
    normalized_plan_ref = _normalize_text_value(plan_ref)
    resolved_file, artifact_path = _resolve_repo_artifact_path(ctx, body_file)
    if not resolved_file.is_file():
        raise typer.BadParameter(f"Plan file must be a file: {body_file}")
    if resolved_file.suffix.lower() != ".md":
        raise typer.BadParameter(f"Plan file must be a Markdown file: {body_file}")
    try:
        markdown = resolved_file.read_text(encoding="utf-8")
    except OSError as exc:
        raise typer.BadParameter(f"Could not read plan file {body_file}: {exc}") from exc
    known_refs = list_plan_section_refs(markdown)
    if normalized_plan_ref is None:
        if len(known_refs) == 1:
            normalized_plan_ref = str(known_refs[0]["plan_ref"])
        elif not known_refs:
            if allow_generic_markdown:
                return {
                    "artifact_path": artifact_path,
                    "artifact_selector": None,
                    "artifact_heading": _default_markdown_artifact_heading(markdown, artifact_path),
                    "items": [],
                    "artifact_body": markdown,
                    "artifact_blob_id": _artifact_blob_id(markdown),
                }
            raise typer.BadParameter(f"{artifact_path} does not expose any `[plan-ref: ...]` section headings yet.")
        else:
            raise typer.BadParameter(
                f"`--plan-ref` is required for {artifact_path} because it exposes multiple `[plan-ref: ...]` sections. "
                f"Known refs: {', '.join(str(entry['plan_ref']) for entry in known_refs)}"
            )
    section = extract_plan_section(markdown, normalized_plan_ref)
    if section is None:
        known_ref_values = [entry["plan_ref"] for entry in known_refs]
        if known_ref_values:
            raise typer.BadParameter(
                f"Plan ref {normalized_plan_ref!r} is not present in {artifact_path}. Known refs: {', '.join(known_ref_values)}"
            )
        raise typer.BadParameter(
            f"{artifact_path} does not expose any `[plan-ref: ...]` section headings yet."
        )
    return {
        "artifact_path": artifact_path,
        "artifact_selector": normalized_plan_ref,
        "artifact_heading": section["heading_title"],
        "items": section["items"],
        "artifact_body": markdown,
        "artifact_blob_id": _artifact_blob_id(markdown),
    }
