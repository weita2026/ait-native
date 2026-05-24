from __future__ import annotations

from ait_protocol.common import find_plan_item_in_items, normalize_optional_text, read_json

from . import local_control
from .repo_paths import RepoContext
from .store_local_views import _local_plan_revision_view
from .store_repo_config import _load_worktree_config, load_config

__all__ = ["current_line", "resolve_local_task_plan_linkage"]


def current_line(ctx: RepoContext) -> str:
    if ctx.worktree_config_path is not None:
        cfg = _load_worktree_config(ctx)
        if cfg.get("current_line"):
            return cfg["current_line"]
    cfg = read_json(ctx.config_path, default={}) or {}
    if isinstance(cfg, dict) and cfg.get("current_line"):
        return cfg["current_line"]
    return local_control.get_meta(ctx, "current_line") or "main"


def resolve_local_task_plan_linkage(
    ctx: RepoContext,
    *,
    plan_id: str | None = None,
    origin_plan_revision_id: str | None = None,
    plan_item_ref: str | None = None,
) -> tuple[str | None, str | None, str | None]:
    repo_name = load_config(ctx).get("repo_name") or ctx.root.name
    resolved_plan_id = normalize_optional_text(plan_id)
    resolved_revision_id = normalize_optional_text(origin_plan_revision_id)
    resolved_plan_item_ref = normalize_optional_text(plan_item_ref)
    if resolved_plan_id is None and resolved_revision_id is None:
        if resolved_plan_item_ref is not None:
            raise ValueError("plan_item_ref requires plan linkage")
        return None, None, None

    plan_row = None
    if resolved_plan_id is not None:
        plan_row = local_control.get_workflow_plan(ctx, resolved_plan_id)
        if str(plan_row["repo_name"]) != repo_name:
            raise KeyError(f"Local plan {resolved_plan_id} belongs to repository {plan_row['repo_name']}, not {repo_name}")

    revision_row = None
    if resolved_revision_id is not None:
        revision_row = local_control.get_workflow_plan_revision_by_id(ctx, resolved_revision_id)

    if plan_row is None and revision_row is not None:
        resolved_plan_id = str(revision_row["plan_id"])
        plan_row = local_control.get_workflow_plan(ctx, resolved_plan_id)
        if str(plan_row["repo_name"]) != repo_name:
            raise KeyError(f"Local plan {resolved_plan_id} belongs to repository {plan_row['repo_name']}, not {repo_name}")
    elif plan_row is not None and revision_row is None:
        resolved_revision_id = normalize_optional_text(plan_row.get("head_revision_id"))
        if resolved_revision_id is None:
            raise ValueError(f"Local plan {resolved_plan_id} has no head revision to link from")
        revision_row = local_control.get_workflow_plan_revision(ctx, resolved_plan_id, resolved_revision_id)

    if plan_row is not None and revision_row is not None and str(revision_row["plan_id"]) != str(plan_row["plan_id"]):
        raise ValueError(f"Local plan revision {resolved_revision_id} does not belong to plan {plan_row['plan_id']}")

    if resolved_plan_item_ref is not None:
        revision = _local_plan_revision_view(revision_row) or {}
        plan_item = find_plan_item_in_items(revision.get("items"), resolved_plan_item_ref)
        if plan_item is None:
            known_refs = [item["plan_item_ref"] for item in revision.get("items") or []]
            if known_refs:
                raise ValueError(
                    f"Plan item ref {resolved_plan_item_ref!r} is not present in local plan revision {resolved_revision_id}. "
                    f"Known refs: {', '.join(known_refs)}"
                )
            raise ValueError(
                f"Local plan revision {resolved_revision_id} does not expose any explicit `[ref: ...]` plan items yet. "
                "Add refs to the file-backed plan section before binding a task to one."
            )
    return resolved_plan_id, resolved_revision_id, resolved_plan_item_ref
