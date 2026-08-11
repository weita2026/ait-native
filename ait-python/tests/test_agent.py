from __future__ import annotations

from pathlib import Path

import pytest

from ait_python import AgentCapabilities, AgentClient, NativeProtocolError


def test_capabilities_come_from_real_rust_binding() -> None:
    capabilities = AgentClient().capabilities()

    assert capabilities.contract == "ait.agent.worker.capabilities.v1"
    assert capabilities.version
    assert set(capabilities.supported_transports) == {
        "telegram",
        "discord",
        "slack",
        "line",
    }
    assert capabilities.default_event_loop_backend in capabilities.event_loop_backends
    assert capabilities.raw["python_worker_execution_allowed"] is False


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


def test_management_lists_empty_manifest_through_real_binding(
    tmp_path: Path,
) -> None:
    (tmp_path / ".ait").mkdir()
    manifest_path = tmp_path / ".ait" / "agent-workers.json"

    assert (
        AgentClient().list_workers(
            "telegram",
            repo_root=tmp_path,
            manifest_path=manifest_path,
        )
        == []
    )


def test_reply_provider_uses_real_worker_transaction_binding() -> None:
    result = AgentClient().reply_provider({"contract": "unsupported"})

    assert result["contract"] == "ait.agent.gateway_reply_provider_response.v1"
    assert result["error"]["kind"] == "provider_request_contract"


def test_agent_input_validation_happens_before_native_call() -> None:
    client = AgentClient()

    with pytest.raises(ValueError, match="unsupported agent transport"):
        client.list_workers("email")
    with pytest.raises(ValueError, match="worker name"):
        client.start("telegram", " ")
    with pytest.raises(ValueError, match="unsupported worker operation"):
        client.worker_transaction("run-command", {})
    with pytest.raises(ValueError, match="unsupported worker operation"):
        client.worker_transaction("graph-watch", {})
    assert not hasattr(client, "telegram_graph_watch")
    with pytest.raises(TypeError, match="unsupported agent context"):
        client.list_workers("telegram", command_args=[])
