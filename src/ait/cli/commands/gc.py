from __future__ import annotations

from ..runtime_inspection_views import _storage_validation_view
from ..shared import export_app_namespace

export_app_namespace(globals())

@gc_app.command("stats")
def gc_stats_cmd(json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    _emit(content_storage_stats(ctx), json_output)


@gc_app.command("validate")
def gc_validate_cmd(json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    _emit(_storage_validation_view(content_storage_stats(ctx)), json_output)


@gc_app.command("pack")
def gc_pack_cmd(
    repack: bool = typer.Option(False, "--repack"),
    max_members: int | None = typer.Option(None, "--max-members"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = local_pack_content(ctx, max_members=max_members, repack=repack)
    except Exception as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@gc_app.command("optimize")
def gc_optimize_cmd(json_output: bool = typer.Option(False, "--json")):
    ctx = _ctx()
    try:
        data = local_optimize_content(ctx)
    except Exception as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@gc_app.command("prune")
def gc_prune_cmd(
    prune_unreferenced: bool = typer.Option(True, "--prune-unreferenced/--no-prune-unreferenced"),
    prune_orphan_packs: bool = typer.Option(True, "--prune-orphan-packs/--no-prune-orphan-packs"),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = local_gc_content(
            ctx,
            prune_unreferenced=prune_unreferenced,
            prune_orphan_packs=prune_orphan_packs,
        )
    except Exception as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)

