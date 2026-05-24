from __future__ import annotations

from ... import local_control
from ..queue_summary_helpers import _queue_summary_payload
from ..workflow_land_batch import (
    _workflow_land_batch_payload as _workflow_land_batch_payload_impl,
    _workflow_land_batch_run as _workflow_land_batch_run_impl,
    _workflow_land_completed_local_apply as _workflow_land_completed_local_apply_impl,
    _workflow_land_completed_local_payload as _workflow_land_completed_local_payload_impl,
)
from ..workflow_land_apply import (
    _workflow_land_apply as _workflow_land_apply_impl,
)
from ..workflow_land_completed_local import (
    _workflow_land_apply_completed_local_entry,
    _workflow_land_completed_local_preview_state,
)
from ..workflow_land_task_dag import (
    _workflow_batch_local_task_dag_session_row,
    _workflow_batch_task_dag_entry_metadata,
    _workflow_land_batch_ensure_remote_task_dag_session,
)
from ..workflow_land_snapshot_replay import (
    _patchset_publish_context,
    _resolve_completed_local_promotion_parent_snapshot_id,
    _workflow_land_batch_ensure_remote_patchset_for_landed_change,
    _workflow_land_batch_ensure_remote_target_line_base,
)
from ..workflow_land_publish import (
    _ensure_local_line_at_snapshot,
    _ensure_patchset_not_empty,
    _guard_patchset_revision_scope,
    _local_snapshot_chain_segment,
    _publish_patchset_from_current_line,
    _sync_patchset_revision_snapshot,
    _workflow_publish_auto_rebase_if_needed,
    _workflow_publish_payload,
    _workflow_publish_slug,
    _workflow_refresh_patchset_for_land,
)
from ..workflow_land_selection import (
    _workflow_batch_local_change_entries as _workflow_batch_local_change_entries_impl,
    _workflow_land_batch_graph_run_selector as _workflow_land_batch_graph_run_selector_impl,
)
from ..workflow_land_state import (
    _workflow_code_review_summary_count,
    _workflow_land_payload,
    _workflow_review_lane_counts,
)
from ..workflow_land_text import (
    _render_workflow_land_text as _render_workflow_land_text_impl,
)
from ..workflow_land_views import (
    _workflow_land_batch_item_status,
    _workflow_land_preview_item_status,
)
from ..shared import export_app_namespace

export_app_namespace(globals())


def _workflow_batch_local_change_entries(
    ctx: RepoContext,
    *,
    remote_name: str,
    local_change_id: str | None = None,
) -> dict[str, Any]:
    return _workflow_batch_local_change_entries_impl(
        ctx,
        remote_name=remote_name,
        local_change_id=local_change_id,
    )




def _workflow_land_batch_graph_run_selector(
    ctx: RepoContext,
    *,
    remote_name: str,
    graph_run_session_id: str,
) -> dict[str, Any]:
    return _workflow_land_batch_graph_run_selector_impl(
        ctx,
        remote_name=remote_name,
        graph_run_session_id=graph_run_session_id,
    )










def _workflow_land_completed_local_payload(
    ctx: RepoContext,
    *,
    change_id: str,
    remote_name: str | None,
) -> dict[str, Any]:
    return _workflow_land_completed_local_payload_impl(
        ctx,
        change_id=change_id,
        remote_name=remote_name,
        selector_completed_local_fn=_workflow_batch_local_change_entries,
    )



def _workflow_land_completed_local_apply(
    ctx: RepoContext,
    *,
    change_id: str,
    remote_name: str | None,
    summary: str | None,
    tests: str | None,
    lint: str | None,
    security: str | None,
    license: str | None,
    author_mode: AuthorMode | None,
    model: str | None,
    session: str | None,
    checkpoint: str | None,
    reviewer: str | None,
    review_message: str | None,
    target: str | None,
    mode: str,
) -> dict[str, Any]:
    return _workflow_land_completed_local_apply_impl(
        ctx,
        change_id=change_id,
        remote_name=remote_name,
        summary=summary,
        tests=tests,
        lint=lint,
        security=security,
        license=license,
        author_mode=author_mode,
        model=model,
        session=session,
        checkpoint=checkpoint,
        reviewer=reviewer,
        review_message=review_message,
        target=target,
        mode=mode,
        selector_completed_local_fn=_workflow_batch_local_change_entries,
        apply_fn=_workflow_land_apply,
    )



def _workflow_land_batch_payload(
    ctx: RepoContext,
    *,
    all_completed_local: bool,
    graph_run_session_id: str | None,
    remote_name: str | None,
    target: str | None,
) -> dict[str, Any]:
    return _workflow_land_batch_payload_impl(
        ctx,
        all_completed_local=all_completed_local,
        graph_run_session_id=graph_run_session_id,
        remote_name=remote_name,
        target=target,
        selector_completed_local_fn=_workflow_batch_local_change_entries,
        graph_run_selector_fn=_workflow_land_batch_graph_run_selector,
    )



def _workflow_land_batch_run(
    ctx: RepoContext,
    *,
    all_completed_local: bool,
    graph_run_session_id: str | None,
    remote_name: str | None,
    summary: str | None,
    tests: str | None,
    lint: str | None,
    security: str | None,
    license: str | None,
    author_mode: AuthorMode | None,
    model: str | None,
    session: str | None,
    checkpoint: str | None,
    reviewer: str | None,
    review_message: str | None,
    target: str | None,
    mode: str,
) -> dict[str, Any]:
    return _workflow_land_batch_run_impl(
        ctx,
        all_completed_local=all_completed_local,
        graph_run_session_id=graph_run_session_id,
        remote_name=remote_name,
        summary=summary,
        tests=tests,
        lint=lint,
        security=security,
        license=license,
        author_mode=author_mode,
        model=model,
        session=session,
        checkpoint=checkpoint,
        reviewer=reviewer,
        review_message=review_message,
        target=target,
        mode=mode,
        selector_completed_local_fn=_workflow_batch_local_change_entries,
        graph_run_selector_fn=_workflow_land_batch_graph_run_selector,
        apply_fn=_workflow_land_apply,
    )



def _workflow_land_apply(
    ctx: RepoContext,
    *,
    change_id: str | None,
    patchset_id: str | None,
    remote_name: str | None,
    snapshot_message: str | None,
    patchset_summary: str | None,
    tests: str | None,
    lint: str | None,
    security: str | None,
    license: str | None,
    author_mode: AuthorMode | None,
    model: str | None,
    session: str | None,
    checkpoint: str | None,
    reviewer: str | None,
    review_message: str | None,
    target: str | None,
    mode: str,
    ignore_workspace_authoring: bool = False,
    patchset_is_authoritative: bool = False,
) -> dict[str, Any]:
    return _workflow_land_apply_impl(
        ctx,
        change_id=change_id,
        patchset_id=patchset_id,
        remote_name=remote_name,
        snapshot_message=snapshot_message,
        patchset_summary=patchset_summary,
        tests=tests,
        lint=lint,
        security=security,
        license=license,
        author_mode=author_mode,
        model=model,
        session=session,
        checkpoint=checkpoint,
        reviewer=reviewer,
        review_message=review_message,
        target=target,
        mode=mode,
        ignore_workspace_authoring=ignore_workspace_authoring,
        patchset_is_authoritative=patchset_is_authoritative,
    )




def _render_workflow_land_text(data: dict[str, Any]) -> str:
    return _render_workflow_land_text_impl(data)
