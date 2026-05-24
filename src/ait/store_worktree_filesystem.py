from __future__ import annotations

import ctypes
import errno
import os
import shutil
import stat
import subprocess
import sys
from pathlib import Path
from typing import Iterable

_LINUX_FICLONE = 0x40049409


def _is_windows_platform() -> bool:
    return sys.platform.startswith("win")


def _path_is_junction(path: Path) -> bool:
    checker = getattr(path, "is_junction", None)
    if checker is None:
        return False
    try:
        return bool(checker())
    except OSError:
        return False


def _path_is_directory_link(path: Path) -> bool:
    return path.is_symlink() or _path_is_junction(path)


def _path_exists_or_directory_link(path: Path) -> bool:
    return path.exists() or _path_is_directory_link(path)


def _create_windows_directory_junction(link_path: Path, target_path: Path) -> None:
    completed = subprocess.run(
        ["cmd", "/c", "mklink", "/J", str(link_path), str(target_path)],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode == 0:
        return
    details = (completed.stderr or completed.stdout or "").strip() or "unknown mklink /J failure"
    raise OSError(f"Failed to create Windows directory junction {link_path} -> {target_path}: {details}")


def _create_directory_link(link_path: Path, target_path: Path) -> None:
    if _is_windows_platform():
        try:
            _create_windows_directory_junction(link_path, target_path)
            return
        except OSError as junction_error:
            try:
                link_path.symlink_to(target_path, target_is_directory=True)
                return
            except OSError as symlink_error:
                raise OSError(
                    f"Failed to create directory link {link_path} -> {target_path}: "
                    f"junction error: {junction_error}; symlink error: {symlink_error}"
                ) from symlink_error
    link_path.symlink_to(target_path, target_is_directory=True)


def _remove_path_entry(path: Path) -> None:
    if path.is_symlink() or path.is_file():
        path.unlink()
        return
    if _path_is_junction(path) or path.is_dir():
        path.rmdir()
        return
    if path.exists():
        path.unlink()


def _remove_tree_entry(path: Path) -> None:
    if _path_is_directory_link(path) or path.is_file():
        _remove_path_entry(path)
        return
    if path.is_dir():
        for child in sorted(path.iterdir(), key=lambda item: (item.is_file(), str(item)), reverse=True):
            _remove_tree_entry(child)
        path.rmdir()
        return
    if path.exists():
        path.unlink()


def _remove_tree_force(path: Path) -> None:
    if not _path_exists_or_directory_link(path):
        return
    if _path_is_directory_link(path) or path.is_file():
        _remove_path_entry(path)
        return
    _make_tree_writable(path)
    shutil.rmtree(path)


def _make_tree_writable(path: Path) -> None:
    for current_root, dirnames, filenames in os.walk(path, topdown=False):
        root_path = Path(current_root)
        for filename in filenames:
            file_path = root_path / filename
            if file_path.is_symlink():
                continue
            mode = file_path.stat().st_mode
            file_path.chmod(mode | stat.S_IWUSR)
        for dirname in dirnames:
            dir_path = root_path / dirname
            if dir_path.is_symlink():
                continue
            mode = dir_path.stat().st_mode
            dir_path.chmod(mode | stat.S_IWUSR | stat.S_IXUSR)
    if path.exists():
        mode = path.stat().st_mode
        path.chmod(mode | stat.S_IWUSR | stat.S_IXUSR)


def _make_tree_readonly(path: Path) -> None:
    for current_root, dirnames, filenames in os.walk(path, topdown=False):
        root_path = Path(current_root)
        for filename in filenames:
            file_path = root_path / filename
            if file_path.is_symlink():
                continue
            mode = file_path.stat().st_mode
            file_path.chmod(mode & ~(stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH))
        for dirname in dirnames:
            dir_path = root_path / dirname
            if dir_path.is_symlink():
                continue
            mode = dir_path.stat().st_mode
            dir_path.chmod((mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH) & ~(stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH))
    if path.exists():
        mode = path.stat().st_mode
        path.chmod((mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH) & ~(stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH))


def _try_clonefile_macos(source_path: Path, target_path: Path) -> bool:
    clonefile = getattr(ctypes.CDLL(None, use_errno=True), "clonefile", None)
    if clonefile is None:
        return False
    result = clonefile(os.fsencode(source_path), os.fsencode(target_path), 0)
    if result == 0:
        shutil.copystat(source_path, target_path, follow_symlinks=False)
        return True
    error_code = ctypes.get_errno()
    if error_code in {errno.ENOTSUP, errno.EXDEV, errno.ENOSYS, errno.EPERM, errno.EINVAL}:
        return False
    raise OSError(error_code, os.strerror(error_code), str(source_path))


def _try_reflink_linux(source_path: Path, target_path: Path) -> bool:
    import fcntl

    source_fd = os.open(source_path, os.O_RDONLY)
    target_fd = os.open(target_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, source_path.stat().st_mode & 0o777)
    try:
        try:
            fcntl.ioctl(target_fd, _LINUX_FICLONE, source_fd)
        except OSError as exc:
            if exc.errno in {errno.EOPNOTSUPP, errno.ENOTTY, errno.EXDEV, errno.EINVAL, errno.ENOSYS, errno.EPERM}:
                return False
            raise
        shutil.copystat(source_path, target_path, follow_symlinks=False)
        return True
    finally:
        os.close(source_fd)
        os.close(target_fd)


def _copy_seed_file(source_path: Path, target_path: Path) -> str:
    target_path.parent.mkdir(parents=True, exist_ok=True)
    if source_path.is_symlink():
        os.symlink(os.readlink(source_path), target_path)
        return "symlink"
    if sys.platform == "darwin":
        if _try_clonefile_macos(source_path, target_path):
            return "clonefile"
    if sys.platform.startswith("linux"):
        try:
            if _try_reflink_linux(source_path, target_path):
                return "reflink"
        except FileExistsError:
            target_path.unlink(missing_ok=True)
            raise
        except OSError:
            if target_path.exists():
                target_path.unlink()
            raise
    shutil.copy2(source_path, target_path)
    return "copy2"


def _copy_seed_tree(
    source_path: Path,
    target_path: Path,
    *,
    exclude_names: Iterable[str] | None = None,
) -> str:
    excluded = {str(name) for name in (exclude_names or ())}
    target_path.mkdir(parents=True, exist_ok=True)
    strategies_used: set[str] = set()
    for root, dirnames, filenames in os.walk(source_path):
        root_path = Path(root)
        rel_root = root_path.relative_to(source_path)
        dirnames[:] = [name for name in dirnames if name not in excluded]
        for dirname in dirnames:
            (target_path / rel_root / dirname).mkdir(parents=True, exist_ok=True)
        for filename in filenames:
            if filename in excluded:
                continue
            source_file = root_path / filename
            target_file = target_path / rel_root / filename
            strategies_used.add(_copy_seed_file(source_file, target_file))
    if "clonefile" in strategies_used:
        return "clonefile"
    if "reflink" in strategies_used:
        return "reflink"
    if "symlink" in strategies_used and len(strategies_used) == 1:
        return "symlink"
    return "copy2"


__all__ = [
    "_copy_seed_file",
    "_copy_seed_tree",
    "_create_directory_link",
    "_create_windows_directory_junction",
    "_is_windows_platform",
    "_make_tree_readonly",
    "_make_tree_writable",
    "_path_exists_or_directory_link",
    "_path_is_directory_link",
    "_path_is_junction",
    "_remove_path_entry",
    "_remove_tree_entry",
    "_remove_tree_force",
    "_try_clonefile_macos",
    "_try_reflink_linux",
]
