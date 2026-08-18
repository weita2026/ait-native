"""In-process Python bindings for the Rust-owned AIT runtime."""

from .agent import AgentCapabilities, AgentClient
from .contract import (
    AGENT_CAPABILITIES_CONTRACT,
    LANGUAGE_BINDING_CONTRACT,
)
from .errors import NativeBridgeError, NativeProtocolError, NativeResolutionError
from .runtime import NativeRuntime

__all__ = [
    "AGENT_CAPABILITIES_CONTRACT",
    "LANGUAGE_BINDING_CONTRACT",
    "AgentCapabilities",
    "AgentClient",
    "NativeBridgeError",
    "NativeProtocolError",
    "NativeResolutionError",
    "NativeRuntime",
]

__version__ = "1.0.0rc13"
