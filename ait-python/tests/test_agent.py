from __future__ import annotations

import pytest

from ait_python import AgentCapabilities, AgentClient, NativeProtocolError


@pytest.mark.parametrize(
    ("field", "value", "message"),
    [
        ("contract", "wrong", "unsupported capabilities contract"),
        ("binary", "wrong", "wrong binary"),
        ("python_worker_execution_allowed", True, "forbidden Python fallback"),
    ],
)
def test_capabilities_fail_closed(
    field: str, value: object, message: str
) -> None:
    payload = {
        "contract": "ait.agent.worker.capabilities.v1",
        "binary": "ait-agent-worker",
        "version": "0.1.0",
        "platform": "test",
        "architecture": "test",
        "supported_transports": ["telegram"],
        "event_loop_backends": ["portable_poll"],
        "default_event_loop_backend": "portable_poll",
        "python_worker_execution_allowed": False,
    }
    payload[field] = value

    with pytest.raises(NativeProtocolError, match=message):
        AgentCapabilities.from_payload(payload)


class UnexpectedRuntime:
    def __getattr__(self, name: str) -> object:
        raise AssertionError(f"retired facade resolved runtime method {name}")


@pytest.mark.parametrize(
    ("method", "args", "kwargs"),
    [
        ("capabilities", (), {}),
        ("manage", ({"operation": "add"},), {}),
        ("add", ({"name": "main"},), {}),
        ("list_workers", ("telegram",), {}),
        ("status", ("telegram",), {}),
        ("start", ("telegram", "main"), {}),
        ("stop", ("telegram", "main"), {}),
        ("restart", ("telegram", "main"), {}),
        ("remove", ("telegram", "main"), {}),
        ("logs", ("telegram", "main"), {}),
        ("worker_transaction", ("slack-command", {}), {}),
        ("slack_command", ({},), {}),
        ("discord_interaction", ({},), {}),
        ("reply_provider", ({},), {}),
        # Retirement precedes legacy validation and environment/path coercion.
        ("list_workers", ("email",), {"repo_root": object()}),
        ("start", ("telegram", " "), {}),
        ("logs", ("telegram", "main"), {"lines": -1}),
        ("worker_transaction", ("run-command", {}), {}),
    ],
)
def test_retired_facades_fail_without_resolving_runtime(
    method: str, args: tuple[object, ...], kwargs: dict[str, object]
) -> None:
    client = AgentClient(UnexpectedRuntime())

    with pytest.raises(RuntimeError, match=r"Chat integrations are retired in AIT 1\.2\.0"):
        getattr(client, method)(*args, **kwargs)


def test_legacy_payload_parser_remains_available() -> None:
    payload = {
        "contract": "ait.agent.worker.capabilities.v1",
        "binary": "ait-agent-worker",
        "version": "1.1.4",
        "platform": "test",
        "architecture": "test",
        "supported_transports": ["telegram"],
        "event_loop_backends": ["portable_poll"],
        "default_event_loop_backend": "portable_poll",
        "python_worker_execution_allowed": False,
    }
    assert AgentCapabilities.from_payload(payload).supported_transports == ("telegram",)
