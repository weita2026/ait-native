from __future__ import annotations

from ait_protocol.common import (
    CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND,
    render_code_review_summary_template,
)

from ..review_submission_helpers import (
    _request_team_review_result,
    _review_action_result,
)
from ..shared import export_app_namespace

export_app_namespace(globals())


review_code_app = typer.Typer(help="Record AI code review evidence for a patchset.")
review_task_app = typer.Typer(help="Record task/outcome review decisions.")
review_team_app = typer.Typer(help="Record preserved team patchset review decisions.")
review_app.add_typer(review_code_app, name="code")
review_app.add_typer(review_task_app, name="task")
review_app.add_typer(review_team_app, name="team")


def _request_team_review(change_id: str, group: list[str], patchset: Optional[str], note: Optional[str], remote: Optional[str], json_output: bool) -> None:
    ctx = _ctx()
    data = _request_team_review_result(
        ctx,
        change_id=change_id,
        group=group,
        patchset=patchset,
        note=note,
        remote=remote,
    )
    _emit(data, json_output)


def _review_action(
    change_id: str,
    reviewer: Optional[str],
    action: str,
    blocking: bool,
    patchset: Optional[str],
    message: Optional[str],
    remote: Optional[str],
    json_output: bool,
) -> None:
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


@review_code_app.command(
    "submit",
    help="Submit the structured AI code review for the selected or current patchset.",
    short_help="Submit AI code review evidence.",
)
def review_code_submit(
    change_id: str,
    verdict: str = typer.Option("pass", "--verdict", help="pass, request-changes, or defer."),
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: str = typer.Option(..., "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    normalized_verdict = str(verdict or "").strip().lower().replace("_", "-")
    if normalized_verdict not in {"pass", "request-changes", "defer"}:
        raise typer.BadParameter("--verdict must be one of: pass, request-changes, defer.")
    if missing_code_review_summary_sections(message):
        raise typer.BadParameter(code_review_summary_requirement_text(message))
    action = "code_review_defer" if normalized_verdict == "defer" else "code_review_summary"
    _review_action(change_id, reviewer, action, normalized_verdict == "request-changes", patchset, message, remote, json_output)


@review_code_app.command(
    "template",
    help="Print a safe code review summary scaffold for agent or human use.",
    short_help="Show code review template.",
)
def review_code_template(
    style: str = typer.Option("numbered", "--style", help="inline or numbered."),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        normalized_style = str(style or "").strip().lower()
        template = render_code_review_summary_template(normalized_style)
    except ValueError as exc:
        raise typer.BadParameter(str(exc)) from exc
    payload = {
        "style": normalized_style,
        "template": template,
        "hint_command": CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND,
    }
    if json_output:
        _emit(payload, True)
    else:
        typer.echo(template)


@review_task_app.command(
    "approve",
    help="Approve the task/outcome result for the selected or current patchset.",
    short_help="Approve task outcome.",
)
def review_task_approve(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: Optional[str] = typer.Option(None, "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _review_action(change_id, reviewer, "task_approve", False, patchset, message, remote, json_output)


@review_task_app.command(
    "request-changes",
    help="Request task/outcome changes for the selected or current patchset.",
    short_help="Request task changes.",
)
def review_task_request_changes(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: Optional[str] = typer.Option(None, "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _review_action(change_id, reviewer, "task_request_changes", True, patchset, message, remote, json_output)


@review_task_app.command(
    "comment",
    help="Record non-blocking task/outcome review commentary.",
    short_help="Comment on task outcome.",
)
def review_task_comment(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: Optional[str] = typer.Option(None, "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _review_action(change_id, reviewer, "task_comment", False, patchset, message, remote, json_output)


@review_task_app.command(
    "defer",
    help="Record that task/outcome review is intentionally deferred.",
    short_help="Defer task review.",
)
def review_task_defer(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: Optional[str] = typer.Option(None, "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _review_action(change_id, reviewer, "task_defer", False, patchset, message, remote, json_output)


@review_team_app.command(
    "request",
    help="Request team review from one or more reviewer groups for the selected or current patchset.",
    short_help="Request team review.",
)
def review_team_request(
    change_id: str,
    group: list[str] = typer.Option(..., "--group"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    note: Optional[str] = typer.Option(None, "--note"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _request_team_review(change_id, group, patchset, note, remote, json_output)


@review_team_app.command(
    "approve",
    help="Record team patchset approval for the selected or current patchset.",
    short_help="Approve team review.",
)
def review_team_approve(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: Optional[str] = typer.Option(None, "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _review_action(change_id, reviewer, "approve", False, patchset, message, remote, json_output)


@review_team_app.command(
    "request-changes",
    help="Record blocking team feedback that requests patchset changes.",
    short_help="Request team changes.",
)
def review_team_request_changes(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: Optional[str] = typer.Option(None, "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _review_action(change_id, reviewer, "request_changes", True, patchset, message, remote, json_output)


@review_team_app.command(
    "comment",
    help="Record non-blocking team review commentary on the selected or current patchset.",
    short_help="Comment on team review.",
)
def review_team_comment(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: Optional[str] = typer.Option(None, "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _review_action(change_id, reviewer, "comment", False, patchset, message, remote, json_output)


@review_team_app.command(
    "defer",
    help="Record that a team reviewer looked but is intentionally deferring a decision.",
    short_help="Defer team review.",
)
def review_team_defer(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: Optional[str] = typer.Option(None, "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _review_action(change_id, reviewer, "defer", False, patchset, message, remote, json_output)


@review_app.command(
    "request",
    help="Request review from one or more reviewer groups for the selected or current patchset on a change. Legacy alias for `ait review team request`.",
    short_help="Request review from reviewer groups.",
    hidden=True,
)
def review_request(
    change_id: str,
    group: list[str] = typer.Option(..., "--group"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    note: Optional[str] = typer.Option(None, "--note"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _request_team_review(change_id, group, patchset, note, remote, json_output)


@review_app.command(
    "show",
    help="Inspect the current approval, blocking, comment, and review-request state for a change.",
    short_help="Inspect review state for a change.",
)
def review_show(change_id: str, remote: Optional[str] = typer.Option(None, "--remote"), json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        data = remote_list_reviews(remote_row["url"], change_id, repo_name=repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    summary = Table(title=f"reviews for {change_id}")
    summary.add_column("current_patchset")
    summary.add_column("approvals")
    summary.add_column("blocking")
    summary.add_column("comments")
    summary.add_row(data.get("current_patchset_id") or "", str(data.get("approvals", 0)), str(data.get("blocking", 0)), str(data.get("comments", 0)))
    rprint(summary)
    if data.get("review_requests"):
        req = Table(title="review requests")
        req.add_column("patchset")
        req.add_column("group")
        req.add_column("note")
        for row in data["review_requests"]:
            req.add_row(row["patchset_id"], row["reviewer_group"], row.get("note") or "")
        rprint(req)
    if data.get("reviews"):
        table = Table(title="review actions")
        table.add_column("reviewer")
        table.add_column("patchset")
        table.add_column("action")
        table.add_column("blocking")
        table.add_column("comment")
        for row in data["reviews"]:
            table.add_row(row["reviewer"], row["patchset_id"], row["action"], str(bool(row["blocking"])), row.get("comment") or "")
        rprint(table)


@review_app.command(
    "approve",
    help="Record a human approval for the selected or current patchset on a change. Legacy alias for `ait review team approve`.",
    short_help="Record team approval for a patchset.",
    hidden=True,
)
def review_approve(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: Optional[str] = typer.Option(None, "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _review_action(change_id, reviewer, "approve", False, patchset, message, remote, json_output)


@review_app.command(
    "request-changes",
    help="Record blocking human feedback that requests changes on the selected or current patchset. Legacy alias for `ait review team request-changes`.",
    short_help="Record blocking review feedback.",
    hidden=True,
)
def review_request_changes(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: Optional[str] = typer.Option(None, "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _review_action(change_id, reviewer, "request_changes", True, patchset, message, remote, json_output)


@review_app.command(
    "comment",
    help="Record non-blocking human review commentary on the selected or current patchset. Legacy alias for `ait review team comment`.",
    short_help="Record non-blocking review commentary.",
    hidden=True,
)
def review_comment(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: Optional[str] = typer.Option(None, "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _review_action(change_id, reviewer, "comment", False, patchset, message, remote, json_output)


@review_app.command(
    "code-summary",
    help="Deprecated alias for `ait review code submit --verdict pass`.",
    short_help="Record code review summary evidence.",
    hidden=True,
)
def review_code_summary(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: str = typer.Option(..., "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    if missing_code_review_summary_sections(message):
        raise typer.BadParameter(code_review_summary_requirement_text(message))
    _review_action(change_id, reviewer, "code_review_summary", False, patchset, message, remote, json_output)


@review_app.command(
    "defer",
    help="Record that a reviewer looked at the selected or current patchset but is explicitly deferring a decision. Legacy alias for `ait review team defer`.",
    short_help="Record an explicit deferred review state.",
    hidden=True,
)
def review_defer(
    change_id: str,
    reviewer: Optional[str] = typer.Option(None, "--reviewer"),
    patchset: Optional[str] = typer.Option(None, "--patchset"),
    message: Optional[str] = typer.Option(None, "--message"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    _review_action(change_id, reviewer, "defer", False, patchset, message, remote, json_output)
