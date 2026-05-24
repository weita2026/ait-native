from __future__ import annotations

import importlib
import os
import sys
from pathlib import Path


def _candidate_workspace_roots() -> list[Path]:
    roots: list[Path] = []
    env_root = os.environ.get("AIT_WORKTREE_PATH")
    if env_root:
        roots.append(Path(env_root))
    cwd = Path.cwd()
    roots.append(cwd)
    roots.extend(cwd.parents)
    return roots


def _preferred_worktree_src() -> Path | None:
    if os.environ.get("AIT_DISABLE_WORKTREE_SOURCE_BOOTSTRAP"):
        return None
    for root in _candidate_workspace_roots():
        marker = root / ".ait-worktree.json"
        src = root / "src"
        app_file = src / "ait" / "cli" / "app.py"
        if marker.is_file() and app_file.is_file():
            return src.resolve()
    return None


def _bootstrap_worktree_source() -> None:
    preferred_src = _preferred_worktree_src()
    if preferred_src is None:
        return
    current_src = Path(__file__).resolve().parents[2]
    if preferred_src == current_src:
        return

    preferred_src_text = str(preferred_src)
    sys.path = [entry for entry in sys.path if entry != preferred_src_text]
    sys.path.insert(0, preferred_src_text)

    ait_pkg = sys.modules.get("ait")
    if ait_pkg is not None:
        ait_pkg.__path__ = [str(preferred_src / "ait")]
        ait_init = preferred_src / "ait" / "__init__.py"
        if ait_init.is_file():
            ait_pkg.__file__ = str(ait_init)

    cli_pkg = sys.modules.get(__name__)
    if cli_pkg is not None:
        cli_pkg.__path__ = [str(preferred_src / "ait" / "cli")]
        cli_pkg.__file__ = str(preferred_src / "ait" / "cli" / "__init__.py")

    importlib.invalidate_caches()


_bootstrap_worktree_source()

from .app import *  # noqa: F401,F403,E402
