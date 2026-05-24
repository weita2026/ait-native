from __future__ import annotations

from ..plan_markdown_authoring import _resolve_plan_artifact_input
from ..shared import export_app_namespace

export_app_namespace(globals())

@plan_session_app.command("create")
def plan_session_create(
    plan_id: str,
    title: Optional[str] = typer.Option(None, "--title"),
    mode: str = typer.Option("connected_local", "--mode"),
    preferred_agent: Optional[str] = typer.Option(None, "--preferred-agent"),
    resume_if_active: bool = typer.Option(True, "--resume-if-active/--no-resume-if-active"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_create_planning_session(
            remote_row["url"],
            plan_id,
            title=title,
            mode=mode,
            preferred_agent=preferred_agent,
            resume_if_active=resume_if_active,
        )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_planning_session_detail(data)


@plan_session_app.command("list")
def plan_session_list(
    plan_id: str,
    status: Optional[str] = typer.Option(None, "--status"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        rows = remote_list_planning_sessions(remote_row["url"], plan_id, status=status)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return
    _render_planning_sessions(plan_id, rows)


@plan_session_app.command("show")
def plan_session_show(
    planning_session_id: str,
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_get_planning_session(remote_row["url"], planning_session_id)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_planning_session_detail(data)


@plan_session_app.command("append")
def plan_session_append(
    planning_session_id: str,
    event_type: str = typer.Option("plan.message", "--type"),
    text: Optional[str] = typer.Option(None, "--text"),
    payload_json: Optional[str] = typer.Option(None, "--payload-json"),
    field: list[str] = typer.Option(None, "--field"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    payload = _parse_json_object_option(payload_json, "--payload-json")
    payload.update(_parse_key_value_options(field, "--field"))
    if text is not None:
        payload["text"] = text
    if not payload:
        raise typer.BadParameter("Provide --text, --payload-json, or --field so the planning event has a payload.")
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_append_planning_session_event(remote_row["url"], planning_session_id, event_type, payload)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@plan_session_app.command("events")
def plan_session_events(
    planning_session_id: str,
    after_sequence: int = typer.Option(0, "--after-sequence"),
    limit: int = typer.Option(200, "--limit"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        rows = remote_list_planning_session_events(
            remote_row["url"],
            planning_session_id,
            after_sequence=after_sequence,
            limit=limit,
        )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return
    _render_planning_session_events(planning_session_id, rows)


@plan_session_app.command("join")
def plan_session_join(
    planning_session_id: str,
    surface: str = typer.Option("cli", "--surface"),
    title: Optional[str] = typer.Option(None, "--title"),
    model_name: Optional[str] = typer.Option(None, "--model-name"),
    resume_if_active: bool = typer.Option(True, "--resume-if-active/--no-resume-if-active"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_join_planning_session(
            remote_row["url"],
            planning_session_id,
            surface=surface,
            title=title,
            model_name=model_name,
            resume_if_active=resume_if_active,
        )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@plan_session_app.command("promote")
def plan_session_promote(
    planning_session_id: str,
    body_file: Optional[Path] = typer.Option(None, "--file", help="Read the promoted plan revision from a Markdown artifact file."),
    plan_ref: Optional[str] = typer.Option(None, "--plan-ref", help="Select the `[plan-ref: ...]` section to promote into the plan head."),
    title: Optional[str] = typer.Option(None, "--title"),
    summary: Optional[str] = typer.Option(None, "--summary"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    plan_artifact = _resolve_plan_artifact_input(ctx, body_file, plan_ref)
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_promote_planning_session(
            remote_row["url"],
            planning_session_id,
            plan_artifact["artifact_path"],
            plan_artifact["artifact_selector"],
            plan_artifact["artifact_heading"],
            plan_artifact["items"],
            title=title,
            summary=summary,
            artifact_body=plan_artifact.get("artifact_body"),
        )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(data, True)
        return
    _render_planning_session_detail(data["planning_session"])
    _render_plan_detail(data["plan"], revision=data.get("promoted_revision"))


@plan_session_app.command("close")
def plan_session_close(
    planning_session_id: str,
    status: str = typer.Option("closed", "--status"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        remote_row, _ = _remote_tuple(ctx, remote)
        data = remote_close_planning_session(remote_row["url"], planning_session_id, status=status)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


