from __future__ import annotations

from ...snapshot_blame import (
    apply_scoped_restore,
    compute_markdown_plan_blame,
    compute_snapshot_blame,
    normalize_blame_path,
    preview_scoped_restore,
    public_blame_payload,
    path_uses_markdown_plan_lineage,
)
from ..shared import export_app_namespace

export_app_namespace(globals())


def _resolve_blame_target(
    ctx: RepoContext,
    *,
    snapshot_id: str | None,
    patchset_id: str | None,
    remote_name: str | None,
    repo_name: str | None,
    change_ref: str | None,
) -> dict[str, Any]:
    if snapshot_id and patchset_id:
        raise ValueError("Choose either --snapshot or --patchset.")
    if patchset_id:
        remote_row, fallback_repo_name = _remote_tuple(ctx, remote_name)
        effective_repo_name = repo_name or fallback_repo_name
        if repo_name and patchset_id.isdigit() and not change_ref:
            raise ValueError("Repo-scoped numeric patchset refs require --change.")
        patchset = remote_get_patchset(
            remote_row["url"],
            patchset_id,
            repo_name=effective_repo_name,
            change_ref=change_ref,
        )
        resolved_snapshot_id = _normalize_text_value(patchset.get("revision_snapshot_id"))
        if resolved_snapshot_id is None:
            raise KeyError(f"Patchset {patchset_id} is missing a revision snapshot id.")
        if not snapshot_exists(ctx, resolved_snapshot_id):
            resolved_patchset_id = str(patchset.get("patchset_id") or patchset_id)
            raise KeyError(
                f"Patchset {resolved_patchset_id} resolved to revision snapshot {resolved_snapshot_id}, "
                "but that snapshot is not available in the local store. Materialize or import the snapshot first."
            )
        change = remote_get_change(remote_row["url"], str(patchset.get("change_id") or ""), repo_name=effective_repo_name)
        return {
            "kind": "patchset",
            "patchset_id": patchset.get("patchset_id"),
            "change_id": patchset.get("change_id"),
            "task_id": change.get("task_id"),
            "base_snapshot_id": patchset.get("base_snapshot_id"),
            "revision_snapshot_id": patchset.get("revision_snapshot_id"),
            "resolved_snapshot_id": resolved_snapshot_id,
        }
    if snapshot_id:
        resolved_snapshot_id = _normalize_text_value(snapshot_id)
        if resolved_snapshot_id is None or not snapshot_exists(ctx, resolved_snapshot_id):
            raise KeyError(f"Unknown snapshot: {snapshot_id}")
        snapshot = get_snapshot(ctx, resolved_snapshot_id)
        return {
            "kind": "snapshot",
            "resolved_snapshot_id": resolved_snapshot_id,
            "line_name": snapshot.get("line_name"),
        }
    line_name = current_line(ctx)
    line_row = get_line(ctx, line_name)
    resolved_snapshot_id = _normalize_text_value(line_row.get("head_snapshot_id"))
    if resolved_snapshot_id is None:
        raise ValueError(f"Current line `{line_name}` has no head snapshot to blame.")
    return {
        "kind": "current_line",
        "line_name": line_name,
        "resolved_snapshot_id": resolved_snapshot_id,
    }


def _public_restore_payload(payload: dict[str, Any]) -> dict[str, Any]:
    public = dict(payload)
    public.pop("_internal", None)
    return public


def _format_hunk(row: dict[str, Any]) -> str:
    start_line = int(row.get("start_line") or 0)
    end_line = int(row.get("end_line") or 0)
    line_label = str(start_line) if start_line == end_line else f"{start_line}-{end_line}"
    owner_id = _normalize_text_value(row.get("snapshot_id")) or _normalize_text_value(row.get("plan_revision_id")) or ""
    details = [owner_id]
    plan_id = _normalize_text_value(row.get("plan_id"))
    if plan_id is not None:
        details.append(f"plan={plan_id}")
    revision_number = row.get("revision_number")
    if revision_number not in {None, ""}:
        details.append(f"revision={revision_number}")
    for key, label in (
        ("task_id", "task"),
        ("change_id", "change"),
        ("patchset_id", "patchset"),
        ("land_id", "land"),
        ("submission_id", "submission"),
        ("session_id", "session"),
        ("checkpoint_id", "checkpoint"),
    ):
        value = _normalize_text_value(row.get(key))
        if value is not None:
            details.append(f"{label}={value}")
    confidence = _normalize_text_value(row.get("provenance_confidence"))
    if confidence is not None:
        details.append(f"confidence={confidence}")
    return f"{line_label:<9} " + " ".join(details)


def _render_human_blame(payload: dict[str, Any], *, restore: dict[str, Any] | None = None) -> None:
    target = payload.get("target") if isinstance(payload.get("target"), dict) else {}
    lines = list(payload.get("hunks") or [])
    warnings = [warning for warning in list(payload.get("warnings") or []) if isinstance(warning, dict)]
    header: list[str] = []
    target_kind = str(target.get("kind") or "current_line")
    if target_kind == "patchset":
        header.append(f"target: patchset {target.get('patchset_id')}")
        if target.get("change_id"):
            header.append(f"change: {target.get('change_id')}")
        if target.get("base_snapshot_id"):
            header.append(f"base: {target.get('base_snapshot_id')}")
        if target.get("revision_snapshot_id"):
            header.append(f"revision: {target.get('revision_snapshot_id')}")
    elif target_kind == "snapshot":
        header.append(f"target: snapshot {payload.get('resolved_snapshot_id')}")
    elif target_kind == "markdown_plan":
        header.append(f"target: markdown plan {target.get('plan_id')}")
        if target.get("resolved_plan_revision_id"):
            header.append(f"revision: {target.get('resolved_plan_revision_id')}")
    else:
        header.append(f"target: current line {payload.get('line_name')}")
        header.append(f"resolved snapshot: {payload.get('resolved_snapshot_id')}")
    header.append(f"path: {payload.get('path')}")
    selected_range = payload.get("range") if isinstance(payload.get("range"), dict) else {}
    if selected_range.get("start"):
        header.append(f"range: {selected_range.get('start')}-{selected_range.get('end')}")
    typer.echo("\n".join(header))
    typer.echo("")
    if lines:
        typer.echo("\n".join(_format_hunk(row) for row in lines))
    else:
        typer.echo("no blameable lines")
    if warnings:
        typer.echo("")
        for warning in warnings:
            message = str(warning.get("message") or "").strip()
            if message:
                typer.echo(f"warning: {message}")
    if restore is None:
        return
    typer.echo("")
    preview_lines = [
        f"selected range: {restore['selected_range']['start']}-{restore['selected_range']['end']}",
        f"restore mode: {restore.get('restore_mode')}",
        f"source snapshot: {restore.get('source_snapshot_id')}",
        "unchanged outside selected range: yes" if restore.get("unchanged_outside_selected_range") else "unchanged outside selected range: no",
        "would overwrite selected local edits: yes" if restore.get("would_overwrite_selected_local_edits") else "would overwrite selected local edits: no",
    ]
    if "applied" in restore:
        preview_lines.append("applied: yes" if restore.get("applied") else "applied: no")
    typer.echo("\n".join(preview_lines))


@app.command(
    "blame",
    help="Attribute file content to the last snapshot that changed each line or hunk on the selected ancestry chain.",
    short_help="Attribute file lines to snapshot ancestry.",
)
def blame_cmd(
    path: str,
    line: int | None = typer.Option(None, "--line", min=1, help="Return blame for one line only."),
    start: int | None = typer.Option(None, "--start", min=1, help="Start line for a bounded blame range."),
    end: int | None = typer.Option(None, "--end", min=1, help="End line for a bounded blame range."),
    restore: bool = typer.Option(False, "--restore", help="Restore only the selected line or range back into the current workspace file."),
    dry_run: bool = typer.Option(False, "--dry-run", help="Preview the scoped restore without writing the workspace file."),
    snapshot_id: str | None = typer.Option(None, "--snapshot", help="Blame against one explicit immutable snapshot."),
    patchset_id: str | None = typer.Option(None, "--patchset", help="Resolve one published patchset to its revision snapshot before blaming."),
    remote: str | None = typer.Option(None, "--remote", help="Remote to use when resolving --patchset."),
    repo: str | None = typer.Option(None, "--repo", help="Resolve repo-scoped patchset refs within this remote repository."),
    change: str | None = typer.Option(None, "--change", help="Required with repo-scoped numeric patchset refs."),
    plan_id: str | None = typer.Option(
        None,
        "--plan-id",
        help="Select one current Markdown lineage plan explicitly when the same artifact path is tracked by multiple current plans.",
    ),
    plan_ref: str | None = typer.Option(
        None,
        "--plan-ref",
        help="Select one current Markdown lineage plan by artifact selector/ref.",
    ),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    if dry_run and not restore:
        raise typer.BadParameter("`--dry-run` is only valid together with `--restore`.")
    if restore and line is None and start is None and end is None:
        raise typer.BadParameter("`--restore` requires `--line` or `--start/--end`.")
    if (repo or change) and not patchset_id:
        raise typer.BadParameter("`--repo` and `--change` are only valid with `--patchset`.")
    try:
        markdown_lineage = path_uses_markdown_plan_lineage(ctx, path)
        target = None
        if markdown_lineage:
            if snapshot_id or patchset_id:
                raise ValueError(
                    f"Path {normalize_blame_path(ctx, path)} is lineage-only Markdown. "
                    "`--snapshot` and `--patchset` are not valid for plan-lineage blame."
                )
            if restore:
                raise ValueError(
                    f"Path {normalize_blame_path(ctx, path)} is lineage-only Markdown. "
                    "`--restore` is not supported for plan-lineage blame."
                )
            blame = compute_markdown_plan_blame(
                ctx,
                path,
                line=line,
                start_line=start,
                end_line=end,
                plan_id=plan_id,
                plan_ref=plan_ref,
            )
        else:
            if plan_id or plan_ref:
                raise ValueError(
                    f"Path {normalize_blame_path(ctx, path)} uses snapshot lineage. "
                    "`--plan-id` and `--plan-ref` are only valid for lineage-only Markdown blame."
                )
            target = _resolve_blame_target(
                ctx,
                snapshot_id=snapshot_id,
                patchset_id=patchset_id,
                remote_name=remote,
                repo_name=repo,
                change_ref=change,
            )
            blame = compute_snapshot_blame(
                ctx,
                path,
                target=target,
                line=line,
                start_line=start,
                end_line=end,
            )
        restore_payload = None
        if restore:
            if dry_run:
                restore_payload = preview_scoped_restore(
                    ctx,
                    path,
                    target=target,
                    line=line,
                    start_line=start,
                    end_line=end,
                )
            else:
                restore_payload = _run_locked_workspace_command(
                    ctx,
                    "blame restore",
                    lambda: apply_scoped_restore(
                        ctx,
                        path,
                        target=target,
                        line=line,
                        start_line=start,
                        end_line=end,
                    ),
                )
    except (KeyError, RemoteError, ValueError, WorkspaceCommandBusyError, FileNotFoundError, IsADirectoryError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    if json_output:
        payload = public_blame_payload(blame)
        if restore_payload is not None:
            payload["restore"] = _public_restore_payload(restore_payload)
        _emit(payload, True)
        return
    _render_human_blame(
        public_blame_payload(blame),
        restore=_public_restore_payload(restore_payload) if restore_payload is not None else None,
    )
