from __future__ import annotations

from pathlib import Path

import ait_py
import pytest

from ait_python import NativeRuntime


RETIRED_CALLS = [
    ("ait_agent_worker_capabilities", RuntimeError),
    ("ait_agent_management", RuntimeError),
    ("ait_agent_worker_transaction", RuntimeError),
    ("ait_agent_web_runtime_execute", ValueError),
    ("ait_agent_env_file_load", ValueError),
    ("ait_agent_telegram_workflow_notification_format", ValueError),
    ("ait_agent_telegram_turn_input_plan", ValueError),
    ("ait_agent_telegram_message_delivery_execute", ValueError),
    ("ait_agent_telegram_workflow_query_plan", ValueError),
]


def test_installed_extension_retains_core_and_shared_helpers() -> None:
    runtime = NativeRuntime()

    info = runtime.binding_info()
    assert info["contract"] == "ait.language.binding.v1"
    assert info["supported_surfaces"] == ["ait-core"]
    assert info["process_transport_allowed"] is False
    assert runtime.call("transport_envelope_ir_version")
    assert isinstance(runtime.call("transport_envelope_schema"), dict)
    assert runtime.call("runtime_binding_state_ir_version")
    assert isinstance(runtime.call("runtime_binding_state_default_payload"), dict)
    assert runtime.call("worker_manifest_ir_version")
    assert isinstance(runtime.call("worker_manifest_default_config"), dict)
    assert runtime.call("plan_filesystem_normalize_markdown_artifact_path", "docs/plan.md") == "docs/plan.md"
    for name in (
        "task_workflow_task_land",
        "task_workflow_task_list",
        "plan_application_build_list",
    ):
        assert callable(runtime.resolve_callable(name))


@pytest.mark.parametrize(("name", "error_type"), RETIRED_CALLS)
def test_installed_retired_exports_preserve_error_channels_and_user_data(
    tmp_path: Path, name: str, error_type: type[Exception]
) -> None:
    user_data = tmp_path / "user-data"
    user_data.mkdir()
    manifest = user_data / "agent-workers.json"
    manifest.write_text('{"preserve": true}', encoding="utf-8")
    env_file = user_data / ".env"
    env_file.write_text("TOKEN=preserve\n", encoding="utf-8")
    before = {path.relative_to(tmp_path): path.read_bytes() for path in user_data.iterdir()}
    request = {
        "operation": "start",
        "transport": "telegram",
        "name": "main",
        "repo_root": str(user_data),
        "cwd": str(user_data),
        "manifest_path": str(manifest),
        "env_file_path": str(env_file),
        "worker_binary": "/must-not-execute/ait-agent-worker",
    }
    export = getattr(ait_py, name)
    assert callable(export)
    args = () if name == "ait_agent_worker_capabilities" else (request,)
    with pytest.raises(error_type, match=r"Chat integrations are retired in AIT 1\.2\.0"):
        export(*args)
    assert {path.relative_to(tmp_path): path.read_bytes() for path in user_data.iterdir()} == before


@pytest.mark.parametrize(("name", "error_type"), RETIRED_CALLS[1:])
def test_native_retirement_precedes_request_parsing(
    name: str, error_type: type[Exception]
) -> None:
    with pytest.raises(error_type, match=r"Chat integrations are retired in AIT 1\.2\.0"):
        getattr(ait_py, name)(object())
