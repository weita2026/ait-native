from __future__ import annotations

import builtins
import importlib
import importlib.util
from pathlib import Path
import sys

import pytest

from ait_python import NativeProtocolError, NativeResolutionError, NativeRuntime, __version__


def test_direct_pyo3_binding_info_and_version() -> None:
    runtime = NativeRuntime()

    payload = runtime.binding_info()

    assert payload["contract"] == "ait.language.binding.v1"
    assert payload["runtime_authority"] == "rust"
    assert payload["python_binding"] == "pyo3"
    assert payload["process_transport_allowed"] is False
    assert runtime.version() == payload["version"] == __version__ == "1.2.0"


def test_generic_call_uses_installed_ait_py_export() -> None:
    runtime = NativeRuntime()

    assert runtime.call("language_binding_info") == runtime.binding_info()
    assert callable(runtime.resolve_callable("ait_agent_management"))
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


@pytest.mark.parametrize(
    ("method", "args"),
    [
        ("agent_capabilities", ()),
        ("agent_management", ({"operation": "start"},)),
        ("agent_worker_transaction", ({"operation": "reply-provider"},)),
    ],
)
def test_retired_runtime_helpers_do_not_load_an_extension(
    monkeypatch: pytest.MonkeyPatch, method: str, args: tuple[object, ...]
) -> None:
    def unexpected_import(*_args: object, **_kwargs: object) -> object:
        raise AssertionError("retired helper imported an extension")

    monkeypatch.setattr(importlib, "import_module", unexpected_import)
    with pytest.raises(RuntimeError, match=r"Chat integrations are retired in AIT 1\.2\.0"):
        getattr(NativeRuntime(), method)(*args)


def test_facade_import_does_not_probe_native_chat(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import ait_python

    package_path = Path(ait_python.__file__)
    package_name = "_ait_core_import_test"
    spec = importlib.util.spec_from_file_location(package_name, package_path)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    original_import = builtins.__import__

    def reject_native_import(name: str, *args: object, **kwargs: object) -> object:
        if name == "ait_py" or name.startswith("ait_py."):
            raise AssertionError("facade import loaded the native module")
        return original_import(name, *args, **kwargs)

    monkeypatch.setattr(builtins, "__import__", reject_native_import)
    monkeypatch.setitem(sys.modules, package_name, module)
    try:
        spec.loader.exec_module(module)
        assert module.NativeRuntime().module_name == "ait_py"
        assert callable(module.AgentCapabilities.from_payload)
    finally:
        for name in tuple(sys.modules):
            if name.startswith(package_name + "."):
                del sys.modules[name]
