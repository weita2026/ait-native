from __future__ import annotations

from ..shared import export_app_namespace
from ..workflow_authoring import _validate_local_scope

export_app_namespace(globals())

@session_app.command("create")
def session_create(
    kind: str = typer.Option("agent_run", "--kind"),
    title: Optional[str] = typer.Option(None, "--title"),
    objective: Optional[str] = typer.Option(None, "--objective"),
    task: Optional[str] = typer.Option(None, "--task"),
    change: Optional[str] = typer.Option(None, "--change"),
    line: Optional[str] = typer.Option(None, "--line"),
    worktree: Optional[str] = typer.Option(None, "--worktree"),
    author_mode: AuthorMode | None = typer.Option(None, "--author-mode"),
    model: Optional[str] = typer.Option(None, "--model"),
    metadata_json: Optional[str] = typer.Option(None, "--metadata-json"),
    meta: list[str] = typer.Option(None, "--meta"),
    local: bool = typer.Option(False, "--local", help="Create a local session instead of creating it on the remote server."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    requested_line = _normalize_text_value(line)
    resolved_model = _effective_model_name(ctx, model)
    metadata = _parse_json_object_option(metadata_json, "--metadata-json")
    metadata.update(_parse_key_value_options(meta, "--meta"))
    if objective is not None:
        metadata["objective"] = objective
    metadata["author_mode"] = _effective_author_mode(ctx, author_mode)
    try:
        _validate_local_scope(local, remote)
        resolved_worktree = _session_bound_worktree(
            ctx,
            local=local,
            remote_name=remote,
            task_id=task,
            change_id=change,
            worktree_name=worktree,
        )
        resolved_line = (
            requested_line
            or _normalize_text_value((resolved_worktree or {}).get("current_line"))
            or _normalize_text_value((resolved_worktree or {}).get("registered_line_name"))
            or current_line(ctx)
        )
        resolved_worktree_name = _normalize_text_value(worktree) or _normalize_text_value((resolved_worktree or {}).get("name"))
        metadata = _apply_session_workspace_metadata(ctx, metadata, worktree=resolved_worktree)
        if local:
            data = create_local_session(
                ctx,
                kind,
                task_id=task,
                change_id=change,
                title=title,
                line_name=resolved_line,
                worktree_name=resolved_worktree_name,
                model_name=resolved_model,
                metadata=metadata,
            )
        else:
            remote_row, repo_name = _sync_remote_repository_defaults(ctx, remote)
            data = remote_create_session(
                remote_row["url"],
                repo_name,
                kind,
                task_id=task,
                change_id=change,
                title=title,
                line_name=resolved_line,
                worktree_name=resolved_worktree_name,
                model_name=resolved_model,
                metadata=metadata,
            )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if resolved_worktree is not None:
        _touch_worktree_usage_safely(ctx, name=str(resolved_worktree.get("name") or ""))
    _emit(data, json_output)


@session_app.command("list")
def session_list(
    status: Optional[str] = typer.Option(None, "--status"),
    local: bool = typer.Option(False, "--local", help="List local sessions from .ait/control.db."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        _validate_local_scope(local, remote)
        if local:
            rows = list_local_sessions(ctx, status=status)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            rows = remote_list_sessions(remote_row["url"], repo_name, status=status)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return
    _render_sessions(rows)


@session_app.command("show")
def session_show(
    session_id: str,
    local: bool = typer.Option(False, "--local", help="Read the session from the local control store."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        _validate_local_scope(local, remote)
        if local:
            data = get_local_session(ctx, session_id)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            data = remote_get_session(remote_row["url"], session_id, repo_name=repo_name)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@session_app.command("turn")
def session_turn(
    session_id: Optional[str] = typer.Argument(None),
    text: str = typer.Option(..., "--text", help="User message text for the live reply turn."),
    surface: Optional[str] = typer.Option(None, "--surface", help="Client surface label such as vscode or editor."),
    title: Optional[str] = typer.Option(None, "--title", help="Client/session title for the live reply request."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    resolved_session_id = _effective_session_id(session_id)
    if resolved_session_id is None:
        raise typer.BadParameter(
            "Provide SESSION_ID or set AIT_SESSION_ID. Task tracking no longer supplies a default live-turn session."
        )
    resolved_surface = _detected_editor_surface(surface)
    resolved_title = _default_session_turn_title(resolved_surface, title)
    try:
        remote_target = _resolve_session_turn_remote_target(ctx, resolved_session_id, remote_name=remote)
        guard_message = _compact_dag_worker_session_turn_guard(
            remote_target.get("session"),
            remote_name=_normalize_text_value(remote_target.get("remote_name")),
        )
        if guard_message is not None:
            raise ValueError(guard_message)
        remote_row = remote_target["remote"]
        data = remote_create_session_turn(
            remote_row["url"],
            resolved_session_id,
            text=text,
            surface=resolved_surface,
            title=resolved_title,
            repo_name=_normalize_text_value(remote_target.get("repo_name")),
        )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if not bool(data.get("ok")):
        raise click.ClickException(str(data.get("error") or f"Live reply turn failed for {resolved_session_id}."))
    if json_output:
        _emit(data, True)
        return
    reply_text = str(data.get("reply_text") or "").strip()
    if reply_text:
        typer.echo(reply_text)


@session_app.command("append")
def session_append(
    session_id: str,
    event_type: str = typer.Option("session.message", "--type"),
    text: Optional[str] = typer.Option(None, "--text"),
    payload_json: Optional[str] = typer.Option(None, "--payload-json"),
    field: list[str] = typer.Option(None, "--field"),
    local: bool = typer.Option(False, "--local", help="Append the event to the local control store."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    payload = _parse_json_object_option(payload_json, "--payload-json")
    payload.update(_parse_key_value_options(field, "--field"))
    if text is not None:
        payload["text"] = text
    if not payload:
        raise typer.BadParameter("Provide --text, --payload-json, or --field so the event has a payload.")
    ctx = _ctx()
    try:
        _validate_local_scope(local, remote)
        if local:
            data = append_local_session_event(ctx, session_id, event_type, payload)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            data = remote_append_session_event(remote_row["url"], session_id, event_type, payload, repo_name=repo_name)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@session_app.command("events")
def session_events(
    session_id: str,
    after_sequence: int = typer.Option(0, "--after-sequence"),
    limit: int = typer.Option(200, "--limit"),
    local: bool = typer.Option(False, "--local", help="List local session events from .ait/control.db."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        _validate_local_scope(local, remote)
        if local:
            rows = list_local_session_events(ctx, session_id, after_sequence=after_sequence, limit=limit)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            rows = remote_list_session_events(
                remote_row["url"],
                session_id,
                after_sequence=after_sequence,
                limit=limit,
                repo_name=repo_name,
            )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return
    _render_session_events(session_id, rows)


@session_app.command("analyze")
def session_analyze(
    session_id: str,
    after_sequence: int = typer.Option(0, "--after-sequence"),
    limit: int = typer.Option(500, "--limit"),
    local: bool = typer.Option(False, "--local", help="Analyze local session events from .ait/control.db."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        _validate_local_scope(local, remote)
        if local:
            rows = list_local_session_events(ctx, session_id, after_sequence=after_sequence, limit=limit)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            rows = remote_list_session_events(
                remote_row["url"],
                session_id,
                after_sequence=after_sequence,
                limit=limit,
                repo_name=repo_name,
            )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    analysis = _analyze_session_ait_usage(session_id, rows, after_sequence=after_sequence, limit=limit)
    if json_output:
        _emit(analysis, True)
        return
    _render_session_analysis(session_id, analysis)


@session_app.command("checkpoint")
def session_checkpoint(
    session_id: str,
    summary: str = typer.Option(..., "--summary"),
    snapshot: Optional[str] = typer.Option(None, "--snapshot"),
    based_on: Optional[int] = typer.Option(None, "--based-on"),
    decision: list[str] = typer.Option(None, "--decision"),
    next_action: list[str] = typer.Option(None, "--next-action"),
    resume_json: Optional[str] = typer.Option(None, "--resume-json"),
    context: list[str] = typer.Option(None, "--context"),
    local: bool = typer.Option(False, "--local", help="Write the checkpoint into the local control store."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    resume_payload = _parse_json_object_option(resume_json, "--resume-json")
    if decision:
        resume_payload["decisions"] = list(decision)
    if next_action:
        resume_payload["next_actions"] = list(next_action)
    context_fields = _parse_key_value_options(context, "--context")
    if context_fields:
        existing_context = resume_payload.get("context")
        if existing_context is not None and not isinstance(existing_context, dict):
            raise typer.BadParameter("--resume-json field `context` must be an object when combined with --context.")
        merged_context = dict(existing_context or {})
        merged_context.update(context_fields)
        resume_payload["context"] = merged_context
    ctx = _ctx()
    try:
        _validate_local_scope(local, remote)
        if local:
            data = create_local_checkpoint(
                ctx,
                session_id,
                summary,
                snapshot_id=snapshot,
                resume_payload=resume_payload,
                based_on_sequence=based_on,
            )
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            data = remote_create_session_checkpoint(
                remote_row["url"],
                session_id,
                summary,
                snapshot_id=snapshot,
                resume_payload=resume_payload,
                based_on_sequence=based_on,
                repo_name=repo_name,
            )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@session_app.command("checkpoints")
def session_checkpoints(
    session_id: str,
    local: bool = typer.Option(False, "--local", help="List local checkpoints from .ait/control.db."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        _validate_local_scope(local, remote)
        if local:
            rows = list_local_checkpoints(ctx, session_id)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            rows = remote_list_session_checkpoints(remote_row["url"], session_id, repo_name=repo_name)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        _emit(rows, True)
        return
    _render_checkpoints(session_id, rows)


@session_app.command("checkpoint-show")
def session_checkpoint_show(
    checkpoint_id: str,
    local: bool = typer.Option(False, "--local", help="Read the checkpoint from the local control store."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        _validate_local_scope(local, remote)
        if local:
            data = get_local_checkpoint(ctx, checkpoint_id)
        else:
            remote_row, _ = _remote_tuple(ctx, remote)
            data = remote_get_session_checkpoint(remote_row["url"], checkpoint_id)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@session_app.command("resume")
def session_resume(
    session_id: str,
    after_sequence: Optional[int] = typer.Option(None, "--after-sequence"),
    limit: int = typer.Option(200, "--limit"),
    local: bool = typer.Option(False, "--local", help="Resume from the local control store."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        _validate_local_scope(local, remote)
        if local:
            data = resume_local_session(ctx, session_id, after_sequence=after_sequence, limit=limit)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            data = remote_resume_session(
                remote_row["url"],
                session_id,
                after_sequence=after_sequence,
                limit=limit,
                repo_name=repo_name,
            )
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@session_app.command("close")
def session_close(
    session_id: str,
    status: str = typer.Option("paused", "--status", help="Session status: paused, completed, or canceled."),
    local: bool = typer.Option(False, "--local", help="Close the local session instead of the remote session."),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        _validate_local_scope(local, remote)
        if local:
            data = close_local_session(ctx, session_id, status)
        else:
            remote_row, repo_name = _remote_tuple(ctx, remote)
            data = remote_close_session(remote_row["url"], session_id, status=status, repo_name=repo_name)
    except (KeyError, RemoteError, ValueError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)
