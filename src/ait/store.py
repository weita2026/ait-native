from __future__ import annotations

import json
import textwrap
import time
from pathlib import Path
from typing import Any, Iterable, Optional

from .command_profiling import _command_profile_elapsed_ms, _command_profile_record_phase
from . import local_content, local_control
from ait_protocol.common import (
    LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX,
    find_plan_item_in_items,
    generate_namespaced_workflow_id,
    lane_from_risk,
    normalize_author_mode,
    normalize_optional_text,
    policy_profile,
    utc_now,
    workflow_origin_namespace_prefix,
    read_json,
    write_json,
)
from .repo_paths import APP_DIR, CONFIG_NAME, RepoContext, WORKTREE_CONFIG_NAME
from .store_repo_config import (
    _load_worktree_config,
    _save_worktree_config,
    effective_id_namespace_prefix,
    load_config,
    load_policy,
    save_config,
    save_policy,
)
from .store_local_views import (
    _local_change_view,
    _local_plan_revision_view,
    _local_plan_view,
    _local_release_view,
)
from .store_content_ops import (
    content_storage_stats,
    export_snapshot_bundle,
    gc_content,
    import_snapshot_bundle,
    optimize_content,
    pack_content,
)
from .store_workspace_replay import (
    replay_change,
    replay_snapshot,
    revert_change,
    revert_snapshot,
)
from .store_stash import (
    apply_stash,
    create_stash,
    drop_stash,
    get_stash,
    list_stashes,
)
from .store_workspace_restore import (
    restore_workspace,
    restore_workspace_paths,
)
from .store_worktree_runtime import (
    DEFAULT_LINE_CLEANUP_OLDER_THAN,
    DEFAULT_WORKTREE_CLEANUP_OLDER_THAN,
    DEFAULT_WORKTREE_CLEANUP_POLICY,
    DEFAULT_WORKTREE_CREATION_KIND,
    LINE_CLEANUP_CLASSES,
    WORKTREE_CLEANUP_CLASSES,
    WORKTREE_CLEANUP_POLICIES,
    WORKTREE_CREATION_KINDS,
    _coerce_datetime,
    _default_cleanup_policy_for_creation_kind,
    _normalize_older_than,
    _normalize_worktree_cleanup_policy,
    _normalize_worktree_creation_kind,
    archive_line,
    create_line,
    create_snapshot,
    current_line,
    get_remote,
    list_lines,
    set_line_head,
    switch_line,
    workspace_status,
)
from .task_worktree_layout import detect_init_task_worktree_defaults

_REPO_GOVERNANCE_DOCS = (
    ("docs/plan.md", "\n"),
    ("docs/milestone.md", "\n\n"),
)
_REPO_BOOTSTRAP_DIRS = ("docs/sprints",)


def _repo_agents_bootstrap(repo_name: str) -> str:
    return textwrap.dedent(
        f"""\
        # AGENTS

        Status: bootstrap instructions for agents in this `ait`-managed repository.
        Scope: workflow routing until the repository authors narrower local governance.

        ## Workspace Identity

        - This workspace is managed by `ait`.
        - Treat `ait` workflow state as the primary repository operating model.
        - Prefer `ait` workflow commands over raw Git for normal repository work.
        - Use raw Git only for exceptional interoperability or last-resort diagnostics.

        ## Session Bootstrap

        At the start of a new session in this repository:

        1. Read this file.
        2. Read [docs/plan.md](./docs/plan.md).
        3. If command routing is still unclear, run `ait workflow guide inventory` or `ait workflow guide land`.
        4. If this repository includes repo-root `ait-native.md` and you are choosing or switching `workflow_mode`, read it before picking the next workflow path.

        ## Command Routing

        - `ait init` starts this repository in a local-first `solo_local` posture by default.
        - For workflow inventory, prefer `ait queue summary` or `ait queue summary --all-changes`.
        - For one task's readiness, prefer `ait task audit <task-id>`.
        - When opening a task together with its first reviewable change, prefer `ait task start`.
        - For one change's landing path, prefer `ait workflow land <change-id>`.
        - For Markdown lineage, prefer `ait plan sync <file-or-dir>`.
        - If the repository later adds narrower local workflow docs, follow those narrower docs.

        ## Default Local-First Path

        - Start with local-only workflow unless shared durability or shared review is intentionally needed.
        - Keep sprint artifacts under `docs/sprints/`.
        - Do not add, route, or `ait plan sync` sprint entry through `docs/sprints/README.md`; keep sprint routing on the constitutional -> legal-layer -> command-layer path.
        - Common first steps:
          - `ait workflow guide inventory`
          - `ait task start --local --title "Describe the work" --intent "Explain the outcome" --base-line main`
          - `ait snapshot create --message "bootstrap"`

        ## Optional Remote Promotion

        - Stay local-first until a remote and workflow mode are intentionally configured.
        - For the common solo-remote path, use:
          - `ait remote add origin <url> --repo-name {repo_name} --default`
          - `ait config set --workflow-mode solo_remote`
          - `ait plan sync docs/plan.md --remote origin`
        """
    )


def _repo_native_bootstrap(repo_name: str, default_line: str) -> str:
    return textwrap.dedent(
        f"""\
        # ait native

        - default mode after `ait init`: `solo_local`

        ## Choose a mode

        - `solo_local`: keep work local and land with `ait workflow land-local`
        - `solo_remote`: use shared Markdown / task / change / review / land workflow

        ## Switch modes

        - `ait config set --workflow-mode solo_local`
        - `ait config set --workflow-mode solo_remote`

        Switching `workflow_mode` changes future command defaults only. It does not migrate existing plan / task / change lineage.

        ## Next steps

        - local first task:
          - `ait task start --local --title "Describe the work" --intent "Explain the outcome" --base-line {default_line}`
          - `ait snapshot create --message "bootstrap"`
        - solo_remote setup:
          - `ait remote add origin <url> --repo-name {repo_name} --default`
          - `ait config set --workflow-mode solo_remote`
          - `ait plan sync docs/plan.md --remote origin`
          - `ait task start --title "Describe the work" --intent "Explain the outcome" --base-line {default_line}`

        ## Read next

        - `AGENTS.md`
        - `docs/plan.md`
        - `ait workflow guide inventory`
        - `ait workflow guide land`
        """
    )


def _ensure_repo_governance_bootstrap(root: Path, repo_name: str, default_line: str) -> None:
    for relative_path in _REPO_BOOTSTRAP_DIRS:
        path = root / relative_path
        path.mkdir(parents=True, exist_ok=True)
    bootstrap_files = (
        ("AGENTS.md", _repo_agents_bootstrap(repo_name)),
        ("ait-native.md", _repo_native_bootstrap(repo_name, default_line)),
        *_REPO_GOVERNANCE_DOCS,
    )
    for relative_path, content in bootstrap_files:
        path = root / relative_path
        path.parent.mkdir(parents=True, exist_ok=True)
        if path.exists():
            continue
        path.write_text(content, encoding="utf-8")
def _ensure_dirs(ait_dir: Path) -> None:
    (ait_dir / "objects" / "manifests").mkdir(parents=True, exist_ok=True)
    (ait_dir / "objects" / "packs").mkdir(parents=True, exist_ok=True)
    (ait_dir / "objects" / "tree-packs").mkdir(parents=True, exist_ok=True)
    (ait_dir / "refs" / "lines").mkdir(parents=True, exist_ok=True)
    (ait_dir / "workspace").mkdir(parents=True, exist_ok=True)
    (ait_dir / "worktrees").mkdir(parents=True, exist_ok=True)



def init_repo(
    root: Path,
    repo_name: Optional[str],
    default_line: str,
    policy_profile_name: str = "prototype",
    default_author_mode: str = "ai_with_human_review",
    default_model: str | None = None,
) -> RepoContext:
    root = root.resolve()
    ait_dir = root / APP_DIR
    if ait_dir.exists():
        raise FileExistsError(f"{ait_dir} already exists")
    ait_dir.mkdir(exist_ok=True)
    _ensure_dirs(ait_dir)

    ctx = RepoContext(
        root=root,
        ait_dir=ait_dir,
        content_db_path=ait_dir / "content.db",
        control_db_path=ait_dir / "control.db",
        config_path=ait_dir / CONFIG_NAME,
    )
    existing_config = load_config(ctx) if ctx.config_path.exists() else {}
    resolved_repo_name = str(existing_config.get("repo_name") or repo_name or root.name)
    resolved_default_line = str(existing_config.get("default_line") or default_line)
    local_control.initialize(ctx, resolved_repo_name, resolved_default_line)
    local_content.initialize(ctx, resolved_default_line)

    if not ctx.policy_path.exists():
        save_policy(ctx, policy_profile(policy_profile_name))

    model_name = default_model.strip() if isinstance(default_model, str) else None
    if model_name == "":
        model_name = None
    config = dict(existing_config)
    config["repo_name"] = resolved_repo_name
    config["default_line"] = resolved_default_line
    config["current_line"] = str(config.get("current_line") or resolved_default_line)
    config.setdefault("default_remote", None)
    config.setdefault("id_namespace_prefix", "")
    config.setdefault("policy_profile", load_policy(ctx)["policy_id"])
    config["default_author_mode"] = normalize_author_mode(
        str(config.get("default_author_mode") or default_author_mode)
    )
    if "task_worktree" not in config:
        detected_task_worktree_defaults = detect_init_task_worktree_defaults(ctx)
        if detected_task_worktree_defaults:
            config["task_worktree"] = detected_task_worktree_defaults
    if model_name and "default_model" not in config:
        config["default_model"] = model_name
    save_config(ctx, config)

    local_control.record_event(
        ctx,
        "repository.initialized",
        "repository",
        resolved_repo_name,
        {
            "repo_name": resolved_repo_name,
            "default_line": resolved_default_line,
            "content_db": str(ctx.content_db_path),
            "control_db": str(ctx.control_db_path),
        },
    )
    _ensure_repo_governance_bootstrap(root, resolved_repo_name, resolved_default_line)
    return ctx



def add_remote(ctx: RepoContext, name: str, url: str, repo_name: Optional[str], make_default: bool = False) -> dict:
    row = local_control.add_remote(ctx, name, url, repo_name, make_default=make_default)
    if make_default:
        cfg = load_config(ctx)
        cfg["default_remote"] = name
        save_config(ctx, cfg)
    return row



def list_remotes(ctx: RepoContext) -> list[dict]:
    return local_control.list_remotes(ctx)
from .store_line_cleanup import (
    _build_line_usage_indexes,
    _empty_line_usage_summary,
    _line_cleanup_decision,
    _line_cleanup_profile,
    _line_usage_summary,
    _line_usage_summary_from_indexes,
    _normalize_line_cleanup_kind,
    cleanup_lines,
    list_line_cleanup_candidates,
)

# Worktree lifecycle, rebase, sync, and cleanup helpers now live in
# `ait.store_worktrees` and are re-exported below to preserve the
# historical `ait.store` facade.



def get_line(ctx: RepoContext, name: Optional[str] = None) -> dict:
    return local_content.get_line(ctx, name or current_line(ctx))



def iter_workspace_files(root: Path) -> Iterable[Path]:
    return local_content.iter_workspace_files(root)



def snapshot_exists(ctx: RepoContext, snapshot_id: str) -> bool:
    return local_content.snapshot_exists(ctx, snapshot_id)



def list_snapshots(ctx: RepoContext) -> list[dict]:
    return local_content.list_snapshots(ctx)



def get_snapshot(ctx: RepoContext, snapshot_id: str) -> dict:
    return local_content.get_snapshot(ctx, snapshot_id)

def move_ref(ctx: RepoContext, line_name: str, snapshot_id: str) -> dict:
    if not snapshot_exists(ctx, snapshot_id):
        raise KeyError(f"Unknown snapshot: {snapshot_id}")
    return set_line_head(ctx, line_name, snapshot_id)


def ref_history(
    ctx: RepoContext,
    name: str | None = None,
    *,
    limit: int = 20,
) -> dict[str, Any]:
    resolved_name = str(name or f"lines/{current_line(ctx)}").strip()
    if not resolved_name.startswith("lines/"):
        raise ValueError("Only lines/* refs are supported in this starter.")
    if limit < 1:
        raise ValueError("--limit must be at least 1")
    line_name = resolved_name.split("/", 1)[1]
    line_row = get_line(ctx, line_name)
    current_target_snapshot_id = normalize_optional_text(line_row.get("head_snapshot_id"))

    snapshots: list[dict[str, Any]] = []
    if current_target_snapshot_id is not None:
        chain = collect_snapshot_chain(ctx, current_target_snapshot_id)
        for position_from_head, snapshot_id in enumerate(reversed(chain)):
            if position_from_head >= limit:
                break
            snapshot = get_snapshot(ctx, snapshot_id)
            snapshots.append(
                {
                    "snapshot_id": snapshot["snapshot_id"],
                    "parent_snapshot_id": snapshot.get("parent_snapshot_id"),
                    "created_at": snapshot.get("created_at"),
                    "message": snapshot.get("message"),
                    "file_count": snapshot.get("file_count"),
                    "position_from_head": position_from_head,
                    "is_current_target": snapshot_id == current_target_snapshot_id,
                }
            )

    move_events = []
    for row in local_control.list_line_events(ctx, line_name, limit=limit):
        payload = row.get("payload") if isinstance(row.get("payload"), dict) else {}
        move_events.append(
            {
                "event_id": row.get("event_id"),
                "event_type": row.get("event_type"),
                "created_at": row.get("created_at"),
                "name": resolved_name,
                "line_name": line_name,
                "target_snapshot_id": normalize_optional_text(payload.get("head_snapshot_id")),
                "previous_target_snapshot_id": normalize_optional_text(payload.get("previous_head_snapshot_id")),
            }
        )

    return {
        "repo_name": load_config(ctx).get("repo_name") or ctx.root.name,
        "name": resolved_name,
        "line_name": line_name,
        "line_status": line_row.get("status") or "active",
        "current_target_snapshot_id": current_target_snapshot_id,
        "limit": limit,
        "snapshot_count": len(snapshots),
        "move_event_count": len(move_events),
        "snapshots": snapshots,
        "move_events": move_events,
    }



def repo_status(ctx: RepoContext) -> dict:
    from .store_worktrees import list_worktrees, worktree_doctor_from_rows

    repo_name = local_control.get_meta(ctx, "repo_name") or ctx.root.name
    current = current_line(ctx)
    remote_count = len(list_remotes(ctx))
    local_content_started = time.perf_counter_ns()
    data = local_content.repo_status(ctx, repo_name, current, remote_count)
    _command_profile_record_phase(
        tracker=None,
        name="local_content.repo_status",
        value={
            "total": _command_profile_elapsed_ms(local_content_started),
            "workspace_delta": data.get("phase_timings_ms"),
        },
    )
    worktree_doctor_started = time.perf_counter_ns()
    worktree_rows = list_worktrees(ctx, refresh_status=False)
    worktree_hygiene = worktree_doctor_from_rows(worktree_rows)
    _command_profile_record_phase(
        tracker=None,
        name="worktree_doctor",
        value=_command_profile_elapsed_ms(worktree_doctor_started),
    )
    line_cleanup_started = time.perf_counter_ns()
    line_hygiene = list_line_cleanup_candidates(
        ctx,
        older_than=DEFAULT_LINE_CLEANUP_OLDER_THAN,
        include_protected=False,
        worktree_rows=worktree_rows,
    )
    _command_profile_record_phase(
        tracker=None,
        name="list_line_cleanup_candidates",
        value=_command_profile_elapsed_ms(line_cleanup_started),
    )
    data["is_worktree"] = ctx.is_worktree
    data["worktree_name"] = _load_worktree_config(ctx).get("worktree_name") if ctx.is_worktree else None
    data["worktree_hygiene"] = {
        "total_count": worktree_hygiene.get("total_count", 0),
        "stale_count": worktree_hygiene.get("stale_count", 0),
        "cleanup_candidate_count": int(worktree_hygiene.get("safe_auto_remove_count", 0))
        + int(worktree_hygiene.get("safe_cleanup_candidate_count", 0)),
        "manual_review_candidate_count": worktree_hygiene.get("manual_review_candidate_count", 0),
        "protected_count": worktree_hygiene.get("protected_count", 0),
    }
    data["line_hygiene"] = {
        "older_than": line_hygiene.get("older_than"),
        "candidate_count": line_hygiene.get("candidate_count", 0),
        "protected_count": line_hygiene.get("protected_count", 0),
        "inspected_count": line_hygiene.get("inspected_count", 0),
    }
    return data


def collect_snapshot_chain(ctx: RepoContext, snapshot_id: str) -> list[str]:
    return local_content.collect_snapshot_chain(ctx, snapshot_id)



def ensure_snapshot_chain(ctx: RepoContext, bundles: list[dict]) -> list[dict]:
    imported = []
    for bundle in bundles:
        imported.append(import_snapshot_bundle(ctx, bundle))
    return imported


_STORE_WORKTREE_EXPORTS = {
    "_normalize_worktree_name",
    "_update_worktree_registration",
    "abort_worktree_rebase",
    "add_worktree",
    "bind_worktree",
    "cleanup_worktrees",
    "continue_worktree_rebase",
    "ensure_main_seed_mirror",
    "get_worktree",
    "list_worktree_cleanup_candidates",
    "list_worktrees",
    "preview_worktree_rebase",
    "promote_worktree",
    "prune_stale_worktrees",
    "rebase_worktree",
    "recreate_worktree",
    "remove_worktree",
    "remove_worktrees",
    "restore_owned_head",
    "sync_all_worktrees",
    "sync_worktree",
    "touch_worktree_usage",
    "worktree_doctor",
}


def __getattr__(name: str) -> Any:
    if name in _STORE_WORKTREE_EXPORTS:
        from . import store_worktrees as _store_worktrees

        value = getattr(_store_worktrees, name)
        globals()[name] = value
        return value
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")

from . import store_local_workflow as _store_local_workflow

_STORE_LOCAL_WORKFLOW_EXPORTS = (
    "create_local_task",
    "create_local_plan",
    "list_local_plans",
    "get_local_plan",
    "list_local_plan_revisions",
    "get_local_plan_revision",
    "revise_local_plan",
    "close_local_plan",
    "mark_local_plan_published",
    "create_local_release",
    "list_local_releases",
    "get_local_release",
    "update_local_release",
    "list_local_tasks",
    "get_local_task",
    "close_local_task",
    "mark_local_task_published",
    "create_local_change",
    "list_local_changes",
    "get_local_change",
    "close_local_change",
    "land_local_change",
    "mark_local_change_published",
    "_local_actor_identity",
    "create_local_session",
    "list_local_sessions",
    "get_local_session",
    "append_local_session_event",
    "list_local_session_events",
    "create_local_checkpoint",
    "list_local_checkpoints",
    "get_local_checkpoint",
    "resume_local_session",
    "close_local_session",
)

for _name in _STORE_LOCAL_WORKFLOW_EXPORTS:
    globals()[_name] = getattr(_store_local_workflow, _name)

del _name
