from __future__ import annotations

"""Compatibility facade for local-content pack/runtime helpers."""

from . import local_content_pack_runtime as _pack_runtime


_export_names: list[str] = []
for _name in dir(_pack_runtime):
    if _name.startswith("__"):
        continue
    globals()[_name] = getattr(_pack_runtime, _name)
    _export_names.append(_name)

__all__ = tuple(_export_names)

del _name
del _export_names
del _pack_runtime
