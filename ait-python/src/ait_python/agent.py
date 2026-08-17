from __future__ import annotations

from dataclasses import dataclass
import os
from typing import Any, Mapping

from .contract import AGENT_CAPABILITIES_CONTRACT, SUPPORTED_TRANSPORTS
from .errors import NativeProtocolError
from .runtime import NativeRuntime


JsonObject = Mapping[str, Any]
_WORKER_OPERATIONS = frozenset(
    {"slack-command", "discord-interaction", "reply-provider"}
)


@dataclass(frozen=True, slots=True)
class AgentCapabilities:
    contract: str
    version: str
    platform: str
    architecture: str
    supported_transports: tuple[str, ...]
    event_loop_backends: tuple[str, ...]
    default_event_loop_backend: str
    raw: JsonObject

    @classmethod
    def from_payload(cls, payload: Any) -> AgentCapabilities:
        if not isinstance(payload, dict):
            raise NativeProtocolError(
                "ait-agent-worker capabilities must be a JSON object"
            )
        if payload.get("contract") != AGENT_CAPABILITIES_CONTRACT:
            raise NativeProtocolError(
                "ait-agent-worker returned an unsupported capabilities contract"
            )
        if payload.get("binary") != "ait-agent-worker":
            raise NativeProtocolError(
                "ait-agent-worker capabilities identify the wrong binary"
            )
        if payload.get("python_worker_execution_allowed") is not False:
            raise NativeProtocolError(
                "ait-agent-worker capabilities permit a forbidden Python fallback"
            )

        transports = _required_text_list(payload, "supported_transports")
        unknown = set(transports).difference(SUPPORTED_TRANSPORTS)
        if unknown:
            names = ", ".join(sorted(unknown))
            raise NativeProtocolError(
                f"ait-agent-worker reported unsupported transports: {names}"
            )
        backends = _required_text_list(payload, "event_loop_backends")
        default_backend = _required_text(payload, "default_event_loop_backend")
        if default_backend not in backends:
            raise NativeProtocolError(
                "ait-agent-worker default event-loop backend is not available"
            )
        return cls(
            contract=AGENT_CAPABILITIES_CONTRACT,
            version=_required_text(payload, "version"),
            platform=_required_text(payload, "platform"),
            architecture=_required_text(payload, "architecture"),
            supported_transports=transports,
            event_loop_backends=backends,
            default_event_loop_backend=default_backend,
            raw=payload,
        )


@dataclass(frozen=True, slots=True)
class AgentClient:
    runtime: NativeRuntime = NativeRuntime()

    def capabilities(self) -> AgentCapabilities:
        return AgentCapabilities.from_payload(self.runtime.agent_capabilities())

    def worker_transaction(
        self,
        operation: str,
        payload: Any,
        *,
        worker: str = "main",
        **context: Any,
    ) -> Any:
        normalized = operation.strip()
        if normalized not in _WORKER_OPERATIONS:
            choices = ", ".join(sorted(_WORKER_OPERATIONS))
            raise ValueError(
                f"unsupported worker operation {operation!r}; expected: {choices}"
            )
        return self.runtime.agent_worker_transaction(
            {
                "operation": normalized,
                "payload": payload,
                "worker": _worker_name(worker),
                **_context(context),
            }
        )

    def slack_command(
        self, payload: Any, *, worker: str = "main", **context: Any
    ) -> Any:
        return self.worker_transaction(
            "slack-command", payload, worker=worker, **context
        )

    def discord_interaction(
        self, payload: Any, *, worker: str = "main", **context: Any
    ) -> Any:
        return self.worker_transaction(
            "discord-interaction", payload, worker=worker, **context
        )

    def reply_provider(
        self, payload: Any, *, worker: str = "main", **context: Any
    ) -> Any:
        return self.worker_transaction(
            "reply-provider", payload, worker=worker, **context
        )

def _context(values: Mapping[str, Any]) -> dict[str, Any]:
    aliases = {
        "cwd": "cwd",
        "repo_root": "repo_root",
        "manifest_path": "manifest_path",
        "env": "env",
        "signature": "signature",
        "signature_timestamp": "signature_timestamp",
        "now_unix_seconds": "now_unix_seconds",
    }
    unknown = set(values).difference(aliases)
    if unknown:
        names = ", ".join(sorted(unknown))
        raise TypeError(f"unsupported agent context fields: {names}")
    result: dict[str, Any] = {}
    for public_name, request_name in aliases.items():
        if public_name not in values or values[public_name] is None:
            continue
        value = values[public_name]
        if public_name == "env":
            result[request_name] = _environment(value)
        elif public_name == "now_unix_seconds":
            if not isinstance(value, int) or isinstance(value, bool):
                raise TypeError("now_unix_seconds must be an integer")
            result[request_name] = value
        else:
            result[request_name] = os.fspath(value)
    return result


def _environment(value: Any) -> dict[str, str | None]:
    if not isinstance(value, Mapping):
        raise TypeError("env must be a mapping")
    result: dict[str, str | None] = {}
    for name, item in value.items():
        if not isinstance(name, str) or not name:
            raise TypeError("environment override names must be non-empty strings")
        result[name] = None if item is None else os.fspath(item)
    return result


def _worker_name(value: str) -> str:
    normalized = value.strip()
    if not normalized:
        raise ValueError("worker name must not be empty")
    return normalized


def _required_text(payload: JsonObject, field: str) -> str:
    value = payload.get(field)
    if not isinstance(value, str) or not value:
        raise NativeProtocolError(
            f"ait-agent-worker capabilities field {field} must be non-empty text"
        )
    return value


def _required_text_list(payload: JsonObject, field: str) -> tuple[str, ...]:
    value = payload.get(field)
    if (
        not isinstance(value, list)
        or not value
        or any(not isinstance(item, str) or not item for item in value)
        or len(set(value)) != len(value)
    ):
        raise NativeProtocolError(
            f"ait-agent-worker capabilities field {field} must be unique text"
        )
    return tuple(value)
