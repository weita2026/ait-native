from __future__ import annotations

import json
from typing import Any

from ait_protocol.common import (
    LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX,
    find_plan_item_in_items,
    generate_namespaced_workflow_id,
    lane_from_risk,
    utc_now,
    workflow_origin_namespace_prefix,
    read_json,
    write_json,
)
from . import local_control
from .repo_paths import RepoContext, WORKTREE_CONFIG_NAME
from .store_repo_config import (
    _save_worktree_config,
    effective_id_namespace_prefix,
    load_config,
    load_policy,
    save_config,
    save_policy,
)
from .store_bootstrap import init_repo
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
from .store_repo_reads import (
    collect_snapshot_chain,
    ensure_snapshot_chain,
    get_line,
    get_snapshot,
    iter_workspace_files,
    list_snapshots,
    move_ref,
    ref_history,
    repo_status,
    snapshot_exists,
)
from .store_remotes import (
    add_remote,
    get_remote,
    list_remotes,
)
from .store_local_sessions import (
    _local_actor_identity,
    append_local_session_event,
    close_local_session,
    create_local_checkpoint,
    create_local_session,
    get_local_checkpoint,
    get_local_session,
    list_local_checkpoints,
    list_local_session_events,
    list_local_sessions,
    resume_local_session,
)
from .store_local_tasks import (
    close_local_task,
    create_local_task,
    get_local_task,
    list_local_tasks,
    mark_local_task_published,
)
from .store_local_releases import (
    create_local_release,
    get_local_release,
    list_local_releases,
    update_local_release,
)
from .store_local_changes import (
    close_local_change,
    create_local_change,
    get_local_change,
    land_local_change,
    list_local_changes,
    mark_local_change_published,
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
    _set_current_line,
    archive_line,
    create_line,
    create_snapshot,
    current_line,
    list_lines,
    set_line_head,
    switch_line,
    workspace_status,
)
from .task_worktree_layout import detect_init_task_worktree_defaults

# Worktree lifecycle, rebase, sync, and cleanup helpers now live in
# `ait.store_worktrees` and are re-exported below to preserve the
# historical `ait.store` facade.

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
    "create_local_plan",
    "list_local_plans",
    "get_local_plan",
    "list_local_plan_revisions",
    "get_local_plan_revision",
    "revise_local_plan",
    "close_local_plan",
    "mark_local_plan_published",
)

for _name in _STORE_LOCAL_WORKFLOW_EXPORTS:
    globals()[_name] = getattr(_store_local_workflow, _name)

del _name
