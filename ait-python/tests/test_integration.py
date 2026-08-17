from __future__ import annotations

from ait_python import AgentClient, NativeRuntime


def test_installed_extension_exposes_core_and_agent_surfaces() -> None:
    runtime = NativeRuntime()

    assert runtime.binding_info()["contract"] == "ait.language.binding.v1"
    assert runtime.binding_info()["supported_surfaces"] == [
        "ait-core",
        "ait-agent",
        "ait-agent-worker",
    ]
    assert (
        AgentClient(runtime).capabilities().contract
        == "ait.agent.worker.capabilities.v1"
    )
