from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Any

from .server_paths import ServerContext

_TRUTHY = {"1", "true", "yes", "on"}
_FALSY = {"0", "false", "no", "off"}


def _env_flag(name: str) -> bool | None:
    raw = os.environ.get(name)
    if raw is None:
        return None
    value = raw.strip().lower()
    if not value:
        return None
    if value in _FALSY:
        return False
    return value in _TRUTHY or True


def _normalize_host(value: str | None, *, default: str) -> str:
    host = str(value or default).strip()
    if host.startswith("[") and host.endswith("]"):
        host = host[1:-1]
    return host or default


def _is_loopback_host(value: str | None) -> bool:
    host = _normalize_host(value, default="127.0.0.1").lower()
    return host in {"127.0.0.1", "localhost", "::1"} or host.startswith("127.")


def _shared_deployment_detected(*, component: str, server_host: str, web_host: str) -> tuple[bool, str]:
    explicit = _env_flag("AIT_NATIVE_SHARED_DEPLOYMENT")
    if explicit is True:
        return True, "AIT_NATIVE_SHARED_DEPLOYMENT=1"
    if explicit is False:
        return False, "AIT_NATIVE_SHARED_DEPLOYMENT=0"
    if not _is_loopback_host(server_host):
        return True, f"server_host={server_host}"
    return False, "loopback_hosts"


@dataclass(frozen=True)
class SharedRuntimePolicy:
    component: str
    db_backend: str
    deployment_scope: str
    state: str
    ok: bool
    override_active: bool
    override_supported: bool
    reason: str
    server_host: str
    web_host: str

    def as_dict(self) -> dict[str, Any]:
        return {
            "component": self.component,
            "db_backend": self.db_backend,
            "deployment_scope": self.deployment_scope,
            "state": self.state,
            "ok": self.ok,
            "override_active": self.override_active,
            "override_supported": self.override_supported,
            "reason": self.reason,
            "server_host": self.server_host,
            "web_host": self.web_host,
        }


def evaluate_shared_runtime_policy(
    ctx: ServerContext,
    *,
    component: str,
    allow_legacy_override: bool,
) -> SharedRuntimePolicy:
    server_host = _normalize_host(os.environ.get("AIT_NATIVE_SERVER_HOST"), default="127.0.0.1")
    web_host = _normalize_host(os.environ.get("AIT_NATIVE_WEB_HOST"), default="127.0.0.1")
    shared_deployment, detection_reason = _shared_deployment_detected(
        component=component,
        server_host=server_host,
        web_host=web_host,
    )
    if ctx.using_postgres:
        return SharedRuntimePolicy(
            component=component,
            db_backend=ctx.db_backend,
            deployment_scope="shared" if shared_deployment else "local",
            state="postgres_compliant",
            ok=True,
            override_active=False,
            override_supported=False,
            reason="PostgreSQL-backed runtime satisfies the shared deployment policy.",
            server_host=server_host,
            web_host=web_host,
        )
    return SharedRuntimePolicy(
        component=component,
        db_backend=ctx.db_backend,
        deployment_scope="shared" if shared_deployment else "local",
        state="blocked_sqlite_runtime",
        ok=False,
        override_active=False,
        override_supported=False,
        reason=(
            "SQLite-backed ait-server runtime is no longer supported. "
            f"Detection={detection_reason}. ait-server and ait-web must run on PostgreSQL."
        ),
        server_host=server_host,
        web_host=web_host,
    )


def enforce_shared_runtime_policy(
    ctx: ServerContext,
    *,
    component: str,
    allow_legacy_override: bool,
) -> SharedRuntimePolicy:
    policy = evaluate_shared_runtime_policy(
        ctx,
        component=component,
        allow_legacy_override=allow_legacy_override,
    )
    if policy.ok:
        return policy
    raise RuntimeError(policy.reason)
