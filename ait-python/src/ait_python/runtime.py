from __future__ import annotations

from dataclasses import dataclass
import importlib
from types import ModuleType
from typing import Any, Callable, Mapping

from .contract import LANGUAGE_BINDING_CONTRACT
from .errors import NativeProtocolError, NativeResolutionError


DEFAULT_EXTENSION_MODULE = "ait_py"


@dataclass(frozen=True, slots=True)
class NativeRuntime:
    """Resolve and call the installed Rust extension in the current process."""

    module_name: str = DEFAULT_EXTENSION_MODULE

    def load_module(self) -> ModuleType:
        module_name = self.module_name.strip()
        if not module_name:
            raise NativeResolutionError("native extension module name must not be empty")
        try:
            return importlib.import_module(module_name)
        except Exception as error:
            raise NativeResolutionError(
                f"Rust AIT binding is unavailable because `{module_name}` "
                "could not be imported"
            ) from error

    def resolve_callable(self, name: str) -> Callable[..., Any]:
        export_name = name.strip()
        if not export_name:
            raise NativeResolutionError("native extension export name must not be empty")
        module = self.load_module()
        resolved = getattr(module, export_name, None)
        if not callable(resolved):
            raise NativeResolutionError(
                f"Rust AIT binding `{module.__name__}` does not export "
                f"`{export_name}` as a supported function"
            )
        return resolved

    def call(self, name: str, /, *args: Any, **kwargs: Any) -> Any:
        """Call one installed ``ait_py`` export without a process relay."""

        return self.resolve_callable(name)(*args, **kwargs)

    def binding_info(self) -> dict[str, Any]:
        payload = _object_payload(
            self.call("language_binding_info"), "language binding info"
        )
        if payload.get("contract") != LANGUAGE_BINDING_CONTRACT:
            raise NativeProtocolError(
                "ait_py returned an unsupported language binding contract"
            )
        if payload.get("runtime_authority") != "rust":
            raise NativeProtocolError(
                "ait_py language binding does not identify Rust authority"
            )
        if payload.get("python_binding") != "pyo3":
            raise NativeProtocolError(
                "ait_py language binding does not identify PyO3"
            )
        if payload.get("process_transport_allowed") is not False:
            raise NativeProtocolError(
                "ait_py language binding permits a process API transport"
            )
        _required_text(payload, "version", "language binding info")
        return payload

    def version(self) -> str:
        return _required_text(
            self.binding_info(), "version", "language binding info"
        )

    def agent_capabilities(self) -> dict[str, Any]:
        return _object_payload(
            self.call("ait_agent_worker_capabilities"),
            "ait-agent-worker capabilities",
        )

    def agent_worker_transaction(self, request: Mapping[str, Any]) -> Any:
        return self.call("ait_agent_worker_transaction", dict(request))


def _object_payload(payload: Any, label: str) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise NativeProtocolError(f"{label} must be a JSON object")
    return payload


def _required_text(payload: Mapping[str, Any], field: str, label: str) -> str:
    value = payload.get(field)
    if not isinstance(value, str) or not value:
        raise NativeProtocolError(f"{label} field {field} must be non-empty text")
    return value
