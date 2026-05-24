from __future__ import annotations

import sys
from collections.abc import MutableMapping
from typing import Any

_SKIP = {"__builtins__", "__cached__", "__doc__", "__file__", "__loader__", "__name__", "__package__", "__spec__"}

LOCAL_SCOPE_OVERRIDE_HELP = (
    "Force local scope; overrides workflow scope config. "
    "In repositories configured with `workflow_mode=solo_remote`, omitting both "
    "`--local` and `--remote` usually follows the remote-backed default."
)

REMOTE_SCOPE_OVERRIDE_HELP = (
    "Force the selected remote scope; overrides workflow scope config. "
    "In repositories configured with `workflow_mode=solo_remote`, omitting this "
    "usually already follows the remote-backed default."
)

REMOTE_TARGET_DEFAULT_HELP = "Target remote. Defaults to the repository default remote when omitted."

COMPLETED_LOCAL_BATCH_PROMOTION_GUIDANCE = (
    "Completed `solo_local` work should use `ait workflow land <LC-change-id> --remote <name>` "
    "for one landed local change, or `ait workflow land --all-completed-local --remote <name>` "
    "for batch promotion, instead of `task publish` / `change publish`."
)


def _export_from_module(namespace: MutableMapping[str, Any], module: Any) -> None:
    for key, value in vars(module).items():
        if key in _SKIP:
            continue
        namespace.setdefault(key, value)


def export_app_namespace(namespace: MutableMapping[str, Any]) -> None:
    from . import app as _app

    _export_from_module(namespace, _app)
    prefix = f"{__package__}.commands."
    for module_name, module in list(sys.modules.items()):
        if module_name.startswith(prefix) and module is not None:
            _export_from_module(namespace, module)
