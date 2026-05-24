from __future__ import annotations

from ..review_submission_helpers import _review_action_result
from ..shared import export_app_namespace

export_app_namespace(globals())

@land_app.command(
    "submit",
    help="Submit the guarded remote integration step after review, attestation, and policy gates pass.",
    short_help="Submit the guarded remote integration step.",
    hidden=True,
)
def land_submit(
    change_id: str,
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    target: str = typer.Option("main", "--target"),
    mode: str = typer.Option("direct", "--mode"),
    session: Optional[str] = typer.Option(None, "--session", help="Override the hidden session provenance binding used for remote land segmentation."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    repo: Optional[str] = typer.Option(None, "--repo", help="Resolve the change or patchset ref within this remote repository when using repo-scoped workflow identity."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    def _operation():
        try:
            data = _submit_remote_land_with_boundary_event(
                ctx,
                remote_name=remote,
                repo_name_override=repo,
                change_id=change_id,
                patchset_id=patchset,
                target_line=target,
                mode=mode,
                session_id=session,
            )
            data = _attach_local_land_sync(ctx, remote, data)
            data = _maybe_auto_remove_bound_worktree_after_land(
                ctx,
                remote_name=remote,
                change_id=change_id,
                land_result=data,
            )
            return data
        except (KeyError, RemoteError) as exc:
            raise typer.BadParameter(str(exc)) from exc

    try:
        data = _run_locked_task_bound_authoring_command(
            ctx,
            "land submit",
            _operation,
            local=False,
            remote_name=remote,
            change_id=change_id,
        )
    except (ValueError, WorkspaceCommandBusyError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _touch_worktree_usage_safely(ctx)
    _emit(data, json_output)


@land_app.command(
    "show",
    help="Inspect one remote land submission, including its status, result, and any blocker class.",
    short_help="Inspect one remote land submission.",
)
def land_show(
    submission_id: str,
    remote: Optional[str] = typer.Option(None, "--remote"),
    repo: Optional[str] = typer.Option(None, "--repo", help="Resolve the land ref within this remote repository when using repo-scoped workflow identity."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_get_land(remote_row["url"], submission_id, repo_name=repo)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@land_app.command(
    "retry",
    help="Retry a previously blocked or failed remote land submission after its blocker has been cleared.",
    short_help="Retry a blocked or failed land submission.",
)
def land_retry(
    submission_id: str,
    reason: Optional[str] = typer.Option(None, "--reason"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    repo: Optional[str] = typer.Option(None, "--repo", help="Resolve the land ref within this remote repository when using repo-scoped workflow identity."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_retry_land(remote_row["url"], submission_id, reason, repo_name=repo)
        data = _attach_local_land_sync(ctx, remote, data)
        resolved_change_id = str(data.get("change_id") or "").strip()
        if resolved_change_id:
            data = _maybe_auto_remove_bound_worktree_after_land(
                ctx,
                remote_name=remote,
                change_id=resolved_change_id,
                land_result=data,
            )
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _touch_worktree_usage_safely(ctx)
    _emit(data, json_output)


def _review_action(change_id: str, reviewer: Optional[str], action: str, blocking: bool, patchset: Optional[str], message: Optional[str], remote: Optional[str], json_output: bool) -> None:
    ctx = _ctx()
    data = _review_action_result(
        ctx,
        change_id=change_id,
        reviewer=reviewer,
        action=action,
        blocking=blocking,
        patchset=patchset,
        message=message,
        remote=remote,
    )
    _emit(data, json_output)
