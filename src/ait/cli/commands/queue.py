from __future__ import annotations

from ..queue_summary_helpers import _queue_summary_payload
from ..shared import export_app_namespace
from .queue_workflow_land import *  # noqa: F401,F403

export_app_namespace(globals())


@queue_app.command(
    "summary",
    help="Summarize the shared queue and optionally the non-landed change inventory in one helper read.",
    short_help="Summarize helper queue and change inventory.",
)
def queue_summary(
    remote: Optional[str] = typer.Option(None, "--remote"),
    status: str = typer.Option(
        "active",
        "--status",
        help="Remote task status filter: active, completed, abandoned, later_promotion_excluded, canceled, or all.",
    ),
    all_changes: bool = typer.Option(
        False,
        "--all-changes",
        help="Include the full non-landed shared change inventory so one command can answer both queue and change-list questions.",
    ),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = _queue_summary_payload(ctx, remote, status, include_all_changes=all_changes)
    except (KeyError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_queue_summary(data)
