from __future__ import annotations

from ..shared import export_app_namespace

export_app_namespace(globals())

@attest_app.command(
    "put",
    help="Backfill or override patchset evidence when automatic CI/provenance capture is not enough.",
    short_help="Backfill or override patchset evidence.",
    hidden=True,
)
def attest_put(
    patchset_id: Optional[str] = typer.Argument(None),
    change: Optional[str] = typer.Option(None, "--change"),
    tests: str | None = typer.Option(None, "--tests"),
    lint: str | None = typer.Option(None, "--lint"),
    security: str | None = typer.Option(None, "--security"),
    license: str | None = typer.Option(None, "--license"),
    author_mode: AuthorMode | None = typer.Option(None, "--author-mode"),
    model: Optional[str] = typer.Option(None, "--model"),
    session: Optional[str] = typer.Option(None, "--session"),
    checkpoint: Optional[str] = typer.Option(None, "--checkpoint"),
    remote: Optional[str] = typer.Option(None, "--remote"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    evaluation = {}
    if tests is not None:
        evaluation["tests"] = tests
    if lint is not None:
        evaluation["lint"] = lint
    if security is not None:
        evaluation["security_scan"] = security
    if license is not None:
        evaluation["license_scan"] = license
    resolved_author_mode = _effective_author_mode(ctx, author_mode)
    resolved_model = _effective_model_name(ctx, model)
    resolved_session = _effective_session_id(session)
    resolved_checkpoint = _effective_checkpoint_id(checkpoint)
    provenance, detail = build_minimum_provenance(
        resolved_author_mode,
        model_name=resolved_model,
        session_id=resolved_session,
        checkpoint_id=resolved_checkpoint,
    )
    if patchset_id is None and change is None:
        raise typer.BadParameter("Provide PATCHSET_ID or --change so attest put can resolve a patchset.")
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        if patchset_id is None and change is not None:
            patchset_rows = remote_list_patchsets(remote_row["url"], change, repo_name=repo_name)
            if not patchset_rows:
                raise KeyError(f"Change {change} has no patchsets")
            resolved_patchset_id = patchset_rows[0]["patchset_id"]
        else:
            resolved_patchset_id = patchset_id
        assert resolved_patchset_id is not None
        data = remote_put_attestation(
            remote_row["url"],
            resolved_patchset_id,
            resolved_author_mode,
            evaluation,
            provenance,
            detail,
            repo_name=repo_name,
        )
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@attest_app.command(
    "show",
    help="Inspect recorded tests, checks, and provenance evidence for one patchset.",
    short_help="Inspect recorded evidence for a patchset.",
)
def attest_show(patchset_id: str, remote: Optional[str] = typer.Option(None, "--remote"), json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    try:
        remote_row, repo_name = _remote_tuple(ctx, remote)
        data = remote_get_attestation(remote_row["url"], patchset_id, repo_name=repo_name)
    except (KeyError, RemoteError) as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)
