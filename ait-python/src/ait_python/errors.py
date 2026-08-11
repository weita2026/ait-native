class NativeBridgeError(RuntimeError):
    """Base error for deterministic bridge failures."""


class NativeResolutionError(NativeBridgeError):
    """The installed native extension or requested export is unavailable."""


class NativeProtocolError(NativeBridgeError):
    """The native extension returned an invalid bridge payload."""
