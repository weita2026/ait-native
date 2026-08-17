from __future__ import annotations

import pytest

from ait_python import NativeProtocolError, NativeResolutionError, NativeRuntime


def test_direct_pyo3_binding_info_and_version() -> None:
    runtime = NativeRuntime()

    payload = runtime.binding_info()

    assert payload["contract"] == "ait.language.binding.v1"
    assert payload["runtime_authority"] == "rust"
    assert payload["python_binding"] == "pyo3"
    assert payload["process_transport_allowed"] is False
    assert runtime.version() == payload["version"]


def test_generic_call_uses_installed_ait_py_export() -> None:
    runtime = NativeRuntime()

    assert runtime.call("language_binding_info") == runtime.binding_info()
    assert callable(runtime.resolve_callable("ait_agent_worker_transaction"))


def test_missing_module_and_export_fail_closed() -> None:
    with pytest.raises(NativeResolutionError, match="could not be imported"):
        NativeRuntime("ait_py_module_that_does_not_exist").load_module()

    with pytest.raises(NativeResolutionError, match="does not export"):
        NativeRuntime().resolve_callable("not_an_ait_py_export")


def test_binding_payload_validation_is_explicit(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    runtime = NativeRuntime()
    monkeypatch.setattr(NativeRuntime, "call", lambda *_args, **_kwargs: {})

    with pytest.raises(NativeProtocolError, match="unsupported language"):
        runtime.binding_info()
