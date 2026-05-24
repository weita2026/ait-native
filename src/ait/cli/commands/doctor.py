from __future__ import annotations

from pathlib import Path
from typing import Optional

import typer

from ..server_runtime_helpers import postgres_preflight_report
from ..shared import export_app_namespace

from ...local_content import workspace_runtime_root_hygiene

export_app_namespace(globals())


@doctor_app.command("runtime-root")
def doctor_runtime_root_cmd(
    server_data: Optional[Path] = typer.Option(None, "--server-data", help="Server runtime root to validate; defaults to AIT_NATIVE_SERVER_DATA or the platform default."),
    json_output: bool = typer.Option(False, "--json"),
):
    ctx = _ctx()
    try:
        data = workspace_runtime_root_hygiene(ctx.root, runtime_root=server_data)
    except Exception as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)


@doctor_app.command("postgres")
def doctor_postgres_cmd(
    server_data: Optional[Path] = typer.Option(None, "--server-data", help="Server runtime root to validate"),
    backend: str = typer.Option("postgres", "--backend", help="Runtime backend to validate"),
    dsn: Optional[str] = typer.Option(None, "--dsn", help="PostgreSQL DSN override"),
    content_schema: Optional[str] = typer.Option(None, "--content-schema", help="Content schema override"),
    control_schema: Optional[str] = typer.Option(None, "--control-schema", help="Control schema override"),
    connect: bool = typer.Option(False, "--connect", help="Attempt a live PostgreSQL connection"),
    json_output: bool = typer.Option(False, "--json"),
):
    try:
        data = postgres_preflight_report(
            server_data=server_data,
            backend=backend,
            dsn=dsn,
            content_schema=content_schema,
            control_schema=control_schema,
            connect=connect,
        )
    except Exception as exc:
        raise typer.BadParameter(str(exc)) from exc
    _emit(data, json_output)
