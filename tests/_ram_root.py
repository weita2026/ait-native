from __future__ import annotations

from contextlib import contextmanager
import importlib
import shutil
import sys
import time
from pathlib import Path


def detect_host_ram_root() -> Path | None:
    layout = importlib.import_module("ait.task_worktree_layout")
    if sys.platform == "darwin":
        roots = layout._macos_ram_volume_roots()
    elif sys.platform.startswith("linux"):
        roots = layout._linux_detected_memory_roots()
    elif sys.platform.startswith("win"):
        roots = layout._windows_ram_disk_roots()
    else:
        roots = []
    if not roots:
        return None
    return roots[0].resolve()


@contextmanager
def managed_host_ram_root(root: Path):
    auto_root = root / ".ait-repos"
    lock_handle = None
    lock_path = root / ".ait-pytest-ram-root.lock"
    try:
        import fcntl  # type: ignore[attr-defined]
    except ImportError:  # pragma: no cover - non-Unix hosts
        fcntl = None

    if fcntl is not None:
        lock_path.parent.mkdir(parents=True, exist_ok=True)
        lock_handle = lock_path.open("a+", encoding="utf-8")
        deadline = time.monotonic() + 30.0
        while True:
            try:
                fcntl.flock(lock_handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                if time.monotonic() >= deadline:
                    raise TimeoutError(f"Timed out waiting for host RAM root lock: {lock_path}")
                time.sleep(0.05)

    before_children = {child.name for child in auto_root.iterdir() if child.is_dir()} if auto_root.exists() else set()

    try:
        yield root
    finally:
        if auto_root.exists():
            for child in auto_root.iterdir():
                if child.is_dir() and child.name not in before_children:
                    shutil.rmtree(child)
            leaked = sorted(child.name for child in auto_root.iterdir() if child.is_dir() and child.name not in before_children)
            assert not leaked
        if lock_handle is not None and fcntl is not None:
            fcntl.flock(lock_handle.fileno(), fcntl.LOCK_UN)
            lock_handle.close()
            try:
                lock_path.unlink()
            except OSError:
                pass
