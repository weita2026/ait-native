from typing import Any

__all__ = ["app", "main"]


def __getattr__(name: str) -> Any:
    if name == "app":
        from .cli import app as _app

        return _app
    if name == "main":
        from .cli import main as _main

        return _main
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")


def __dir__() -> list[str]:
    return sorted(__all__)
