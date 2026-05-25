from __future__ import annotations

import ctypes
import hashlib
import ntpath
import os
import plistlib
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any

from ait_protocol.common import normalize_optional_text

from .repo_paths import RepoContext, configured_repo_name

DEFAULT_TASK_WORKTREE_ALIAS_ROOT = ".ait/worktree-links"
INTERNAL_WORKTREE_ROOT_DIRNAME = ".ait-internal"
_REPO_SEGMENT_RE = re.compile(r"[^A-Za-z0-9._-]+")
_MACOS_RAM_IMAGE_PREFIX = "ram://"
_AUTO_DETECTED_EPHEMERAL_ROOT_DIRNAME = ".ait-repos"
_MOUNTINFO_ESCAPE_RE = re.compile(r"\\([0-7]{3})")
_LINUX_MEMORY_BACKED_FSTYPES = frozenset({"tmpfs", "ramfs"})
_WINDOWS_DRIVE_RAMDISK = 6


def _repo_path_segment(ctx: RepoContext) -> str:
    repo_name = configured_repo_name(ctx) or ctx.repo_root.name or "repo"
    normalized = _REPO_SEGMENT_RE.sub("-", repo_name).strip("-")
    return normalized or "repo"


def _configured_path(value: Any) -> Path | None:
    text = normalize_optional_text(value)
    if text is None:
        return None
    return Path(text).expanduser()


def resolve_task_worktree_alias_root(ctx: RepoContext, alias_root: Any) -> Path:
    configured = _configured_path(alias_root)
    base = configured if configured is not None else Path(DEFAULT_TASK_WORKTREE_ALIAS_ROOT)
    if not base.is_absolute():
        base = ctx.repo_root / base
    return base.resolve()


def _configured_ephemeral_root(ctx: RepoContext, configured_root: Any) -> Path | None:
    configured = _configured_path(configured_root)
    if configured is None:
        return None
    if not configured.is_absolute():
        configured = ctx.repo_root / configured
    return configured.resolve() / _repo_path_segment(ctx)


def _linux_ephemeral_root_candidates(ctx: RepoContext) -> list[tuple[Path, str]]:
    xdg_runtime_dir = normalize_optional_text(os.environ.get("XDG_RUNTIME_DIR"))
    xdg_runtime_root = Path(xdg_runtime_dir).expanduser().resolve() if xdg_runtime_dir is not None else None
    candidates: list[tuple[Path, str]] = []
    for root in _linux_detected_memory_roots():
        source = "linux_memory_root"
        if xdg_runtime_root is not None and root == xdg_runtime_root:
            source = "linux_xdg_runtime_dir"
        elif root == Path("/dev/shm").resolve():
            source = "linux_dev_shm"
        elif root == Path("/tmp").resolve():
            source = "linux_tmpfs"
        candidates.append((_auto_detected_ephemeral_root(ctx, root) / _repo_path_segment(ctx), source))
    return candidates


def _windows_ephemeral_root_candidates(ctx: RepoContext) -> list[tuple[Path, str]]:
    return [
        (_auto_detected_ephemeral_root(ctx, root) / _repo_path_segment(ctx), "windows_ramdisk")
        for root in _windows_ram_disk_roots()
    ]


def _macos_ephemeral_root_candidates(ctx: RepoContext) -> list[tuple[Path, str]]:
    return [
        (_auto_detected_ephemeral_root(ctx, root) / _repo_path_segment(ctx), "macos_ram_volume")
        for root in _macos_ram_volume_roots()
    ]


def _decode_mountinfo_path(text: str) -> str:
    return _MOUNTINFO_ESCAPE_RE.sub(lambda match: chr(int(match.group(1), 8)), text)


def _nearest_existing_ancestor(path: Path) -> Path | None:
    current = path.expanduser()
    for candidate in (current, *current.parents):
        if candidate.exists():
            return candidate.resolve()
    return None


def _linux_mount_fstype_for_path(path: Path) -> str | None:
    anchor = _nearest_existing_ancestor(path)
    if anchor is None:
        return None
    anchor_text = str(anchor)
    try:
        lines = Path("/proc/self/mountinfo").read_text(encoding="utf-8").splitlines()
    except OSError:
        return None
    best_mount_point: str | None = None
    best_fstype: str | None = None
    for line in lines:
        if " - " not in line:
            continue
        left, right = line.split(" - ", 1)
        left_fields = left.split()
        right_fields = right.split()
        if len(left_fields) < 5 or len(right_fields) < 1:
            continue
        mount_point = _decode_mountinfo_path(left_fields[4])
        if anchor_text != mount_point and not anchor_text.startswith(f"{mount_point.rstrip('/')}/"):
            continue
        if best_mount_point is None or len(mount_point) > len(best_mount_point):
            best_mount_point = mount_point
            best_fstype = right_fields[0]
    return normalize_optional_text(best_fstype)


def _linux_detected_memory_roots() -> list[Path]:
    if not sys.platform.startswith("linux"):
        return []
    candidates: list[Path] = []
    xdg_runtime_dir = normalize_optional_text(os.environ.get("XDG_RUNTIME_DIR"))
    if xdg_runtime_dir is not None:
        candidates.append(Path(xdg_runtime_dir).expanduser())
    candidates.extend([Path("/dev/shm"), Path("/tmp")])

    seen: set[str] = set()
    roots: list[Path] = []
    for candidate in candidates:
        fstype = _linux_mount_fstype_for_path(candidate)
        if fstype not in _LINUX_MEMORY_BACKED_FSTYPES:
            continue
        resolved = candidate.resolve()
        key = str(resolved)
        if key in seen:
            continue
        seen.add(key)
        roots.append(resolved)
    return roots


def _windows_drive_root(path: Path) -> Path | None:
    drive = normalize_optional_text(ntpath.splitdrive(str(path))[0])
    if drive is None:
        return None
    return Path(f"{drive}\\")


def _windows_get_drive_type(root: Path) -> int | None:
    try:
        kernel32 = ctypes.windll.kernel32
    except AttributeError:
        return None
    return int(kernel32.GetDriveTypeW(str(root)))


def _windows_list_drive_roots() -> list[Path]:
    try:
        kernel32 = ctypes.windll.kernel32
    except AttributeError:
        return []
    mask = int(kernel32.GetLogicalDrives())
    roots: list[Path] = []
    for index in range(26):
        if mask & (1 << index) == 0:
            continue
        roots.append(Path(f"{chr(ord('A') + index)}:\\"))
    return roots


def _windows_ram_disk_roots() -> list[Path]:
    if not sys.platform.startswith("win"):
        return []
    raw_candidates: list[Path] = []
    for env_name in ("LOCALAPPDATA", "TEMP", "TMP"):
        value = normalize_optional_text(os.environ.get(env_name))
        if value is None:
            continue
        root = _windows_drive_root(Path(value).expanduser())
        if root is not None:
            raw_candidates.append(root)
    temp_root = _windows_drive_root(Path(tempfile.gettempdir()).expanduser())
    if temp_root is not None:
        raw_candidates.append(temp_root)
    raw_candidates.extend(_windows_list_drive_roots())

    seen: set[str] = set()
    roots: list[Path] = []
    for root in raw_candidates:
        drive_type = _windows_get_drive_type(root)
        if drive_type != _WINDOWS_DRIVE_RAMDISK:
            continue
        resolved = root.resolve()
        key = str(resolved)
        if key in seen:
            continue
        seen.add(key)
        roots.append(resolved)
    return roots


def _macos_ram_volume_roots() -> list[Path]:
    if not sys.platform.startswith("darwin"):
        return []
    try:
        raw = subprocess.check_output(
            ["hdiutil", "info", "-plist"],
            stderr=subprocess.DEVNULL,
        )
    except (FileNotFoundError, OSError, subprocess.CalledProcessError):
        return []
    try:
        payload = plistlib.loads(raw)
    except (plistlib.InvalidFileException, ValueError):
        return []
    images = payload.get("images") if isinstance(payload, dict) else None
    if not isinstance(images, list):
        return []
    seen: set[str] = set()
    roots: list[Path] = []
    for image in images:
        if not isinstance(image, dict):
            continue
        image_path = normalize_optional_text(image.get("image-path"))
        if image_path is None or not image_path.lower().startswith(_MACOS_RAM_IMAGE_PREFIX):
            continue
        if image.get("writeable") is False:
            continue
        entities = image.get("system-entities")
        if not isinstance(entities, list):
            continue
        for entity in entities:
            if not isinstance(entity, dict):
                continue
            mount_point = normalize_optional_text(entity.get("mount-point"))
            if mount_point is None:
                continue
            root = Path(mount_point).expanduser().resolve()
            key = str(root)
            if key in seen:
                continue
            seen.add(key)
            roots.append(root)
    roots.sort(key=str)
    return roots


def _auto_detected_ephemeral_root(ctx: RepoContext, root: Path) -> Path:
    repo_root_hash = hashlib.sha256(str(ctx.repo_root.resolve()).encode("utf-8")).hexdigest()[:12]
    return (root.resolve() / _AUTO_DETECTED_EPHEMERAL_ROOT_DIRNAME / repo_root_hash).resolve()


def detect_init_task_worktree_defaults(ctx: RepoContext) -> dict[str, Any]:
    if sys.platform.startswith("linux"):
        roots = _linux_detected_memory_roots()
        if roots:
            return {"ephemeral_root": str(_auto_detected_ephemeral_root(ctx, roots[0]))}
    if sys.platform.startswith("win"):
        roots = _windows_ram_disk_roots()
        if roots:
            return {"ephemeral_root": str(_auto_detected_ephemeral_root(ctx, roots[0]))}
    if sys.platform.startswith("darwin"):
        roots = _macos_ram_volume_roots()
        if roots:
            return {"ephemeral_root": str(_auto_detected_ephemeral_root(ctx, roots[0]))}
    return {}


def _ensure_root_candidate(path: Path) -> Path | None:
    try:
        path.mkdir(parents=True, exist_ok=True)
    except OSError:
        return None
    return path.resolve()


def _resolve_managed_worktree_root(
    ctx: RepoContext,
    *,
    ephemeral_root: Any,
) -> dict[str, Any]:
    default_root = ctx.task_worktree_dir.resolve()

    configured_root = _configured_ephemeral_root(ctx, ephemeral_root)
    if configured_root is not None:
        target_root = _ensure_root_candidate(configured_root)
        if target_root is not None:
            return {
                "target_root": target_root,
                "root_source": "configured_ephemeral_root",
                "ephemeral_enabled": True,
            }

    if sys.platform.startswith("linux"):
        for candidate, source in _linux_ephemeral_root_candidates(ctx):
            target_root = _ensure_root_candidate(candidate)
            if target_root is None:
                continue
            return {
                "target_root": target_root,
                "root_source": source,
                "ephemeral_enabled": True,
            }

    if sys.platform.startswith("win"):
        for candidate, source in _windows_ephemeral_root_candidates(ctx):
            target_root = _ensure_root_candidate(candidate)
            if target_root is None:
                continue
            return {
                "target_root": target_root,
                "root_source": source,
                "ephemeral_enabled": True,
            }

    if sys.platform.startswith("darwin"):
        for candidate, source in _macos_ephemeral_root_candidates(ctx):
            target_root = _ensure_root_candidate(candidate)
            if target_root is None:
                continue
            return {
                "target_root": target_root,
                "root_source": source,
                "ephemeral_enabled": True,
            }

    return {
        "target_root": default_root,
        "root_source": "repo_internal_fallback",
        "ephemeral_enabled": False,
    }


def resolve_managed_worktree_location(
    ctx: RepoContext,
    *,
    worktree_name: str,
    ephemeral_root: Any,
    alias_root: Any,
) -> dict[str, Any]:
    root_info = _resolve_managed_worktree_root(
        ctx,
        ephemeral_root=ephemeral_root,
    )
    target_root = Path(root_info["target_root"]).resolve()
    target_path = (target_root / worktree_name).resolve()
    alias_path = None
    preferred_path = target_path
    if bool(root_info["ephemeral_enabled"]):
        alias_base = resolve_task_worktree_alias_root(ctx, alias_root)
        alias_path = (alias_base / worktree_name).resolve()
        preferred_path = alias_path
    return {
        "target_path": target_path,
        "alias_path": alias_path,
        "root_source": root_info["root_source"],
        "preferred_path": preferred_path,
        "ephemeral_enabled": bool(root_info["ephemeral_enabled"]),
    }


def resolve_task_auto_worktree_location(
    ctx: RepoContext,
    *,
    worktree_name: str,
    ephemeral_root: Any,
    alias_root: Any,
) -> dict[str, Any]:
    return resolve_managed_worktree_location(
        ctx,
        worktree_name=worktree_name,
        ephemeral_root=ephemeral_root,
        alias_root=alias_root,
    )


def resolve_main_seed_mirror_location(
    ctx: RepoContext,
    *,
    seed_name: str,
    ephemeral_root: Any,
) -> dict[str, Any] | None:
    root_info = _resolve_managed_worktree_root(
        ctx,
        ephemeral_root=ephemeral_root,
    )
    if not bool(root_info["ephemeral_enabled"]):
        return None
    target_root = Path(root_info["target_root"]).resolve()
    target_path = (target_root / INTERNAL_WORKTREE_ROOT_DIRNAME / seed_name).resolve()
    return {
        "target_path": target_path,
        "root_source": root_info["root_source"],
        "preferred_path": target_path,
        "ephemeral_enabled": True,
    }
