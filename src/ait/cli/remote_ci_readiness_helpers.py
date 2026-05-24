from __future__ import annotations

import inspect
import sys
from typing import Any

from ..remote_client import (
    RemoteError,
    get_server_health as remote_get_server_health,
    read_task_dag_readiness as remote_read_task_dag_readiness,
)
from .remote_repository_defaults import _remote_error_status_code


def _app_override(name: str, fallback: Any) -> Any:
    app_module = sys.modules.get("ait.cli.app")
    if app_module is None:
        return fallback
    candidate = getattr(app_module, name, fallback)
    return candidate


def _runtime_ci_capability_payload(base_url: str) -> dict[str, Any] | None:
    get_server_health = _app_override("remote_get_server_health", remote_get_server_health)
    try:
        payload = get_server_health(base_url)
    except RemoteError:
        return None
    if not isinstance(payload, dict):
        return None
    capabilities = payload.get("ci_capabilities")
    readiness = payload.get("ci_readiness")
    return {
        "healthz": payload,
        "ci_capabilities": capabilities if isinstance(capabilities, dict) else None,
        "ci_readiness": readiness if isinstance(readiness, dict) else None,
    }


def _ci_route_mismatch_guidance(
    *,
    base_url: str,
    route_label: str,
    cli_hint: str,
    exc: RemoteError,
) -> str:
    status_code = _remote_error_status_code(exc)
    if status_code not in {404, 405}:
        return str(exc)
    capability_payload = _runtime_ci_capability_payload(base_url)
    capability_hint = ""
    if capability_payload is None:
        capability_hint = (
            "Could not read /healthz from the live runtime, so treat this as a stale or partially updated "
            "ait-server process and restart/update it before retrying."
        )
    else:
        healthz = capability_payload["healthz"]
        capabilities = capability_payload["ci_capabilities"] or {}
        readiness = capability_payload["ci_readiness"] or {}
        runtime_root = str(healthz.get("runtime_root") or "")
        if not capabilities:
            capability_hint = (
                "The live runtime /healthz payload does not advertise ci_capabilities, so this ait-server process "
                "likely predates the native CI routes. Restart/update the live runtime, then retry."
            )
        else:
            route_supported = capabilities.get(route_label)
            generation = str(readiness.get("runtime_generation") or "").strip()
            if route_supported is False:
                capability_hint = (
                    f"/healthz reports ci_capabilities.{route_label}=false, so the running ait-server process does not "
                    "support this CI route yet. Restart/update the live runtime, then retry."
                )
            else:
                capability_hint = (
                    f"/healthz advertises ci_capabilities for {route_label}, but the live runtime still returned "
                    f"{status_code}. Treat this as a stale or partially updated server process and restart/update it "
                    "before retrying."
                )
            if generation:
                capability_hint += f" runtime_generation={generation}."
        if runtime_root:
            capability_hint += f" runtime_root={runtime_root}."
    return (
        f"Live runtime rejected the {route_label} CI route with HTTP {status_code}. "
        f"{capability_hint} Verify support with `{cli_hint}`. Original error: {exc}"
    )


def _readiness_supports_repo_name() -> bool:
    readiness_reader = _app_override("remote_read_task_dag_readiness", remote_read_task_dag_readiness)
    return "repo_name" in inspect.signature(readiness_reader).parameters


def _remote_read_task_dag_readiness(
    base_url: str,
    graph: dict[str, Any],
    *,
    repo_name: str | None = None,
    current_plan_revision_id: str | None = None,
) -> dict[str, Any]:
    readiness_reader = _app_override("remote_read_task_dag_readiness", remote_read_task_dag_readiness)
    kwargs: dict[str, Any] = {}
    if current_plan_revision_id is not None:
        kwargs["current_plan_revision_id"] = current_plan_revision_id
    if repo_name is not None and _readiness_supports_repo_name():
        return readiness_reader(base_url, graph, repo_name=repo_name, **kwargs)
    return readiness_reader(base_url, graph, **kwargs)
