from __future__ import annotations

from contextlib import contextmanager
import importlib
import os
import shutil
import stat
import sys
import time
from collections.abc import Iterable
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


_ACTIVE_ROOT: Path | None = None
_ACTIVE_LOCK_HANDLE = None
_ACTIVE_LOCK_FCNTL = None
_ACTIVE_SNAPSHOTS: list[set[str]] = []


def _make_tree_user_writable(root: Path) -> None:
    for dirpath, dirnames, filenames in os.walk(root, topdown=False, followlinks=False):
        current = Path(dirpath)
        for name in [*filenames, *dirnames]:
            target = current / name
            try:
                mode = os.lstat(target).st_mode
                os.chmod(target, mode | stat.S_IRUSR | stat.S_IWUSR | stat.S_IXUSR, follow_symlinks=False)
            except OSError:
                continue
        try:
            mode = os.lstat(current).st_mode
            os.chmod(current, mode | stat.S_IRUSR | stat.S_IWUSR | stat.S_IXUSR, follow_symlinks=False)
        except OSError:
            continue


def remove_host_ram_root_children(root: Path, child_names: Iterable[str]) -> None:
    auto_root = root.resolve() / ".ait-repos"
    for child_name in child_names:
        child = auto_root / child_name
        if not child.exists():
            continue
        _make_tree_user_writable(child)
        try:
            shutil.rmtree(child)
        except FileNotFoundError:
            continue


@contextmanager
def managed_host_ram_root(root: Path):
    global _ACTIVE_ROOT
    global _ACTIVE_LOCK_FCNTL
    global _ACTIVE_LOCK_HANDLE
    root = root.resolve()
    auto_root = root / ".ait-repos"
    lock_handle = None
    lock_path = root / ".ait-pytest-ram-root.lock"
    release_lock = False
    try:
        import fcntl  # type: ignore[attr-defined]
    except ImportError:  # pragma: no cover - non-Unix hosts
        fcntl = None

    if _ACTIVE_ROOT is not None and _ACTIVE_ROOT != root:
        raise RuntimeError(f"Host RAM root guard already active for {_ACTIVE_ROOT}; cannot switch to {root}.")

    if _ACTIVE_ROOT is None:
        if fcntl is not None:
            lock_path.parent.mkdir(parents=True, exist_ok=True)
            lock_handle = lock_path.open("a+", encoding="utf-8")
            deadline = time.monotonic() + 30.0
            try:
                while True:
                    try:
                        fcntl.flock(lock_handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                        break
                    except BlockingIOError:
                        if time.monotonic() >= deadline:
                            raise TimeoutError(f"Timed out waiting for host RAM root lock: {lock_path}")
                        time.sleep(0.05)
            except Exception:
                lock_handle.close()
                raise
        _ACTIVE_ROOT = root
        _ACTIVE_LOCK_HANDLE = lock_handle
        _ACTIVE_LOCK_FCNTL = fcntl
        release_lock = True

    before_children = {child.name for child in auto_root.iterdir() if child.is_dir()} if auto_root.exists() else set()
    _ACTIVE_SNAPSHOTS.append(before_children)

    try:
        yield root
    finally:
        try:
            snapshot = _ACTIVE_SNAPSHOTS.pop()
            if auto_root.exists():
                new_children = [child for child in auto_root.iterdir() if child.is_dir() and child.name not in snapshot]
                for child in new_children:
                    _make_tree_user_writable(child)
                    try:
                        shutil.rmtree(child)
                    except FileNotFoundError:
                        continue
                leaked = sorted(child.name for child in auto_root.iterdir() if child.is_dir() and child.name not in snapshot)
                assert not leaked, leaked
        finally:
            if release_lock:
                active_handle = _ACTIVE_LOCK_HANDLE
                active_fcntl = _ACTIVE_LOCK_FCNTL
                _ACTIVE_ROOT = None
                _ACTIVE_LOCK_HANDLE = None
                _ACTIVE_LOCK_FCNTL = None
                if active_handle is not None and active_fcntl is not None:
                    active_fcntl.flock(active_handle.fileno(), active_fcntl.LOCK_UN)
                    active_handle.close()
                    try:
                        lock_path.unlink()
                    except OSError:
                        pass
