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
_MACOS_RAM_VOLUME_KIND = "macos_ram_volume"
_LINUX_MEMORY_ROOT_KIND = "linux_memory_root"
_WINDOWS_RAMDISK_KIND = "windows_ramdisk"
_DEFAULT_MACOS_RAM_VOLUME_NAME = "AIT_RAM"
_DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT = 4194304


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


def normalize_task_worktree_memory_root(value: Any) -> dict[str, Any] | None:
    if not isinstance(value, dict):
        return None
    kind = normalize_optional_text(value.get("kind"))
    root = normalize_optional_text(value.get("root"))
    if kind is None or root is None:
        return None
    if kind == _MACOS_RAM_VOLUME_KIND:
        normalized: dict[str, Any] = {
            "kind": kind,
            "root": root,
        }
        volume_name = normalize_optional_text(value.get("volume_name"))
        if volume_name is not None:
            normalized["volume_name"] = volume_name
        sector_count = value.get("sector_count")
        try:
            resolved_sector_count = int(sector_count)
        except (TypeError, ValueError):
            resolved_sector_count = None
        if resolved_sector_count is not None and resolved_sector_count > 0:
            normalized["sector_count"] = resolved_sector_count
        return normalized
    if kind in {_LINUX_MEMORY_ROOT_KIND, _WINDOWS_RAMDISK_KIND}:
        return {
            "kind": kind,
            "root": root,
        }
    return None


def normalize_task_worktree_main_seed_ram_max_bytes(value: Any) -> int | None:
    if value is None or isinstance(value, bool):
        return None
    if isinstance(value, str):
        text = value.strip()
        if not text:
            return None
        value = text
    try:
        resolved = int(value)
    except (TypeError, ValueError):
        return None
    if resolved < 0:
        return None
    return resolved


def main_seed_ram_budget_status(
    ctx: RepoContext,
    *,
    main_seed_ram_max_bytes: Any,
    line_name: str | None = None,
    snapshot_id: str | None = None,
) -> dict[str, Any] | None:
    budget_bytes = normalize_task_worktree_main_seed_ram_max_bytes(main_seed_ram_max_bytes)
    if budget_bytes is None:
        return None
    effective_line_name = normalize_optional_text(line_name)
    if effective_line_name is None:
        from .store_repo_config import load_config

        effective_line_name = normalize_optional_text(load_config(ctx).get("default_line")) or "main"
    effective_snapshot_id = normalize_optional_text(snapshot_id)
    from . import local_content

    if effective_snapshot_id is None:
        try:
            line_row = local_content.get_line(ctx, effective_line_name)
        except KeyError:
            return None
        effective_snapshot_id = normalize_optional_text(line_row.get("head_snapshot_id"))
        if effective_snapshot_id is None:
            return None
    try:
        snapshot = local_content.get_snapshot(ctx, effective_snapshot_id)
    except KeyError:
        return None
    total_bytes = int(snapshot.get("total_bytes") or 0)
    return {
        "default_line": effective_line_name,
        "seed_snapshot_id": effective_snapshot_id,
        "seed_snapshot_total_bytes": total_bytes,
        "main_seed_ram_max_bytes": budget_bytes,
        "exceeded": total_bytes > budget_bytes,
    }


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


def _path_is_relative_to(path: Path, root: Path) -> bool:
    try:
        path.relative_to(root)
    except ValueError:
        return False
    return True


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
    roots: list[Path] = []
    for volume in _macos_ram_volume_specs():
        roots.append(Path(str(volume["root"])).expanduser().resolve())
    return roots


def _macos_ram_volume_specs_from_images(images: list[Any]) -> list[dict[str, Any]]:
    seen: set[str] = set()
    volumes: list[dict[str, Any]] = []
    for image in images:
        if not isinstance(image, dict):
            continue
        image_path = normalize_optional_text(image.get("image-path"))
        if image_path is None or not image_path.lower().startswith(_MACOS_RAM_IMAGE_PREFIX):
            continue
        sector_count_text = image_path[len(_MACOS_RAM_IMAGE_PREFIX) :].strip()
        try:
            sector_count = int(sector_count_text)
        except ValueError:
            sector_count = None
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
            volume: dict[str, Any] = {
                "kind": _MACOS_RAM_VOLUME_KIND,
                "root": str(root),
                "volume_name": root.name,
            }
            if sector_count is not None and sector_count > 0:
                volume["sector_count"] = sector_count
            volumes.append(volume)
    volumes.sort(key=lambda item: str(item["root"]))
    return volumes


def _macos_ram_volume_specs() -> list[dict[str, Any]]:
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
    return _macos_ram_volume_specs_from_images(images)


def _default_macos_ram_volume_spec() -> dict[str, Any]:
    root = (Path("/Volumes") / _DEFAULT_MACOS_RAM_VOLUME_NAME).resolve()
    return {
        "kind": _MACOS_RAM_VOLUME_KIND,
        "root": str(root),
        "volume_name": _DEFAULT_MACOS_RAM_VOLUME_NAME,
        "sector_count": _DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT,
    }


def _infer_macos_auto_detected_memory_root(path: Path) -> dict[str, Any] | None:
    expanded = path.expanduser()
    parts = expanded.parts
    if len(parts) < 5 or parts[0] != "/" or parts[1] != "Volumes" or parts[3] != _AUTO_DETECTED_EPHEMERAL_ROOT_DIRNAME:
        return None
    volume_name = parts[2].strip()
    if not volume_name:
        return None
    volume_root = (Path("/") / "Volumes" / volume_name).resolve()
    return {
        "kind": _MACOS_RAM_VOLUME_KIND,
        "root": str(volume_root),
        "volume_name": volume_name,
        "sector_count": _DEFAULT_MACOS_RAM_VOLUME_SECTOR_COUNT,
    }


def path_is_memory_backed(path: Path | str) -> bool:
    anchor = _nearest_existing_ancestor(Path(path).expanduser())
    if anchor is None:
        return False
    if sys.platform.startswith("linux"):
        return _linux_mount_fstype_for_path(anchor) in _LINUX_MEMORY_BACKED_FSTYPES
    if sys.platform.startswith("darwin"):
        return any(_path_is_relative_to(anchor, root) for root in _macos_ram_volume_roots())
    if sys.platform.startswith("win"):
        drive_root = _windows_drive_root(anchor)
        if drive_root is None:
            return False
        return _windows_get_drive_type(drive_root) == _WINDOWS_DRIVE_RAMDISK
    return False


def _auto_detected_ephemeral_root(ctx: RepoContext, root: Path) -> Path:
    repo_root_hash = hashlib.sha256(str(ctx.repo_root.resolve()).encode("utf-8")).hexdigest()[:12]
    return (root.resolve() / _AUTO_DETECTED_EPHEMERAL_ROOT_DIRNAME / repo_root_hash).resolve()


def derived_task_worktree_ephemeral_root(ctx: RepoContext, memory_root: Any) -> str | None:
    normalized_memory_root = normalize_task_worktree_memory_root(memory_root)
    if normalized_memory_root is None:
        return None
    return str(_auto_detected_ephemeral_root(ctx, _memory_root_path(normalized_memory_root)))


def detect_init_task_worktree_defaults(ctx: RepoContext) -> dict[str, Any]:
    if sys.platform.startswith("linux"):
        roots = _linux_detected_memory_roots()
        if roots:
            return {
                "memory_root": {
                    "kind": _LINUX_MEMORY_ROOT_KIND,
                    "root": str(roots[0]),
                },
            }
    if sys.platform.startswith("win"):
        roots = _windows_ram_disk_roots()
        if roots:
            return {
                "memory_root": {
                    "kind": _WINDOWS_RAMDISK_KIND,
                    "root": str(roots[0]),
                },
            }
    if sys.platform.startswith("darwin"):
        volumes = _macos_ram_volume_specs()
        if volumes:
            return {
                "memory_root": dict(volumes[0]),
            }
    return {}


def infer_task_worktree_memory_root(ctx: RepoContext, ephemeral_root: Any) -> dict[str, Any] | None:
    configured_root = _configured_ephemeral_root(ctx, ephemeral_root)
    if configured_root is None:
        return None
    if sys.platform.startswith("linux"):
        for root in _linux_detected_memory_roots():
            if _path_is_relative_to(configured_root, root):
                return {
                    "kind": _LINUX_MEMORY_ROOT_KIND,
                    "root": str(root),
                }
        return None
    if sys.platform.startswith("win"):
        for root in _windows_ram_disk_roots():
            if _path_is_relative_to(configured_root, root):
                return {
                    "kind": _WINDOWS_RAMDISK_KIND,
                    "root": str(root),
                }
        return None
    if sys.platform.startswith("darwin"):
        for volume in _macos_ram_volume_specs():
            volume_root = Path(str(volume["root"])).expanduser().resolve()
            if _path_is_relative_to(configured_root, volume_root):
                return dict(volume)
        return _infer_macos_auto_detected_memory_root(configured_root)
    return None


def _memory_root_path(spec: dict[str, Any]) -> Path:
    return Path(str(spec["root"])).expanduser()


def _detected_memory_root_from_spec(spec: dict[str, Any]) -> Path | None:
    kind = normalize_optional_text(spec.get("kind"))
    expected_root = _memory_root_path(spec).resolve()
    if kind == _LINUX_MEMORY_ROOT_KIND:
        for root in _linux_detected_memory_roots():
            if root == expected_root:
                return root
        return None
    if kind == _WINDOWS_RAMDISK_KIND:
        for root in _windows_ram_disk_roots():
            if root == expected_root:
                return root
        return None
    if kind == _MACOS_RAM_VOLUME_KIND:
        for volume in _macos_ram_volume_specs():
            root = Path(str(volume["root"])).expanduser().resolve()
            if root == expected_root:
                return root
    return None


def _parse_macos_attached_device(raw: str) -> str | None:
    match = re.search(r"/dev/\S+", raw)
    return match.group(0) if match is not None else None


def _provision_macos_ram_volume(spec: dict[str, Any]) -> bool:
    if normalize_optional_text(spec.get("kind")) != _MACOS_RAM_VOLUME_KIND:
        return False
    volume_name = normalize_optional_text(spec.get("volume_name"))
    sector_count = spec.get("sector_count")
    try:
        resolved_sector_count = int(sector_count)
    except (TypeError, ValueError):
        return False
    if volume_name is None or resolved_sector_count <= 0:
        return False
    if _detected_memory_root_from_spec(spec) is not None:
        return True
    expected_root = _memory_root_path(spec)
    if expected_root.exists():
        return False
    attached_device: str | None = None
    try:
        attach_output = subprocess.check_output(
            ["hdiutil", "attach", "-nomount", f"{_MACOS_RAM_IMAGE_PREFIX}{resolved_sector_count}"],
            stderr=subprocess.DEVNULL,
            text=True,
        )
        attached_device = _parse_macos_attached_device(attach_output)
        if attached_device is None:
            return False
        subprocess.check_call(
            ["diskutil", "erasevolume", "APFS", volume_name, attached_device],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
    except (FileNotFoundError, OSError, subprocess.CalledProcessError):
        if attached_device is not None:
            try:
                subprocess.check_call(
                    ["hdiutil", "detach", attached_device, "-force"],
                    stdout=subprocess.DEVNULL,
                    stderr=subprocess.DEVNULL,
                )
            except (FileNotFoundError, OSError, subprocess.CalledProcessError):
                pass
        return False
    if _detected_memory_root_from_spec(spec) is not None:
        return True
    if attached_device is not None:
        try:
            subprocess.check_call(
                ["hdiutil", "detach", attached_device, "-force"],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        except (FileNotFoundError, OSError, subprocess.CalledProcessError):
            pass
    return False


def _ensure_memory_root_available(spec: dict[str, Any]) -> bool:
    if _detected_memory_root_from_spec(spec) is not None:
        return True
    if normalize_optional_text(spec.get("kind")) == _MACOS_RAM_VOLUME_KIND:
        return _provision_macos_ram_volume(spec)
    return False


def _looks_like_missing_macos_auto_detected_root(path: Path) -> bool:
    if not sys.platform.startswith("darwin"):
        return False
    expanded = path.expanduser()
    parts = expanded.parts
    if len(parts) < 4 or parts[0] != "/" or parts[1] != "Volumes":
        return False
    volume_root = Path("/") / "Volumes" / parts[2]
    if volume_root.exists():
        return False
    return len(parts) >= 5 and parts[3] == _AUTO_DETECTED_EPHEMERAL_ROOT_DIRNAME


def _ensure_configured_ephemeral_root(
    ctx: RepoContext,
    *,
    ephemeral_root: Any,
    memory_root: Any,
    allow_memory_roots: bool = True,
) -> Path | None:
    configured_root = _configured_ephemeral_root(ctx, ephemeral_root)
    if configured_root is None:
        return None
    stored_memory_root = normalize_task_worktree_memory_root(memory_root)
    effective_memory_root = stored_memory_root or infer_task_worktree_memory_root(ctx, ephemeral_root)
    if effective_memory_root is not None:
        if not allow_memory_roots:
            return None
        memory_root_path = _memory_root_path(effective_memory_root)
        if not _path_is_relative_to(configured_root, memory_root_path):
            return None if stored_memory_root is not None else _ensure_root_candidate(configured_root)
        if not _ensure_memory_root_available(effective_memory_root):
            return None
    elif _looks_like_missing_macos_auto_detected_root(configured_root):
        return None
    return _ensure_root_candidate(configured_root)


def _ensure_root_candidate(path: Path) -> Path | None:
    try:
        path.mkdir(parents=True, exist_ok=True)
    except OSError:
        return None
    return path.resolve()


def _collect_macos_resolution_specs(
    ctx: RepoContext,
    *,
    ephemeral_root: Any,
    memory_root: Any,
) -> list[dict[str, Any]]:
    specs: list[dict[str, Any]] = []
    seen: set[str] = set()

    def add(spec: dict[str, Any] | None) -> None:
        normalized = normalize_task_worktree_memory_root(spec)
        if normalized is None or normalize_optional_text(normalized.get("kind")) != _MACOS_RAM_VOLUME_KIND:
            return
        root = _memory_root_path(normalized).resolve()
        key = str(root)
        if key in seen:
            return
        seen.add(key)
        specs.append(normalized)

    add(memory_root if isinstance(memory_root, dict) else None)
    add(infer_task_worktree_memory_root(ctx, ephemeral_root))
    for spec in _macos_ram_volume_specs():
        add(spec)
    add(_default_macos_ram_volume_spec())
    return specs


def _resolve_managed_worktree_root(
    ctx: RepoContext,
    *,
    ephemeral_root: Any,
    memory_root: Any = None,
    main_seed_ram_max_bytes: Any = None,
) -> dict[str, Any]:
    default_root = ctx.task_worktree_dir.resolve()
    normalized_memory_root = normalize_task_worktree_memory_root(memory_root)
    budget_status = main_seed_ram_budget_status(
        ctx,
        main_seed_ram_max_bytes=main_seed_ram_max_bytes,
    )
    memory_roots_allowed = not bool(budget_status and budget_status.get("exceeded"))

    target_root = _ensure_configured_ephemeral_root(
        ctx,
        ephemeral_root=ephemeral_root,
        memory_root=normalized_memory_root,
        allow_memory_roots=memory_roots_allowed,
    )
    if target_root is not None:
        return {
            "target_root": target_root,
            "root_source": "configured_ephemeral_root",
            "ephemeral_enabled": True,
        }

    if memory_roots_allowed and normalized_memory_root is not None and normalize_optional_text(ephemeral_root) is None:
        _ensure_memory_root_available(normalized_memory_root)

    if memory_roots_allowed and sys.platform.startswith("linux"):
        for candidate, source in _linux_ephemeral_root_candidates(ctx):
            target_root = _ensure_root_candidate(candidate)
            if target_root is None:
                continue
            return {
                "target_root": target_root,
                "root_source": source,
                "ephemeral_enabled": True,
            }

    if memory_roots_allowed and sys.platform.startswith("win"):
        for candidate, source in _windows_ephemeral_root_candidates(ctx):
            target_root = _ensure_root_candidate(candidate)
            if target_root is None:
                continue
            return {
                "target_root": target_root,
                "root_source": source,
                "ephemeral_enabled": True,
            }

    if memory_roots_allowed and sys.platform.startswith("darwin"):
        for spec in _collect_macos_resolution_specs(
            ctx,
            ephemeral_root=ephemeral_root,
            memory_root=normalized_memory_root,
        ):
            if not _ensure_memory_root_available(spec):
                continue
            candidate = _auto_detected_ephemeral_root(ctx, _memory_root_path(spec)) / _repo_path_segment(ctx)
            target_root = _ensure_root_candidate(candidate)
            if target_root is None:
                continue
            return {
                "target_root": target_root,
                "root_source": "macos_ram_volume",
                "ephemeral_enabled": True,
            }

    payload = {
        "target_root": default_root,
        "root_source": "repo_internal_fallback",
        "ephemeral_enabled": False,
    }
    if budget_status is not None and bool(budget_status.get("exceeded")):
        payload.update(
            {
                "fallback_reason": "main_seed_ram_budget_exceeded",
                "default_line": budget_status.get("default_line"),
                "seed_snapshot_id": budget_status.get("seed_snapshot_id"),
                "seed_snapshot_total_bytes": budget_status.get("seed_snapshot_total_bytes"),
                "main_seed_ram_max_bytes": budget_status.get("main_seed_ram_max_bytes"),
            }
        )
    return payload


def resolve_managed_worktree_location(
    ctx: RepoContext,
    *,
    worktree_name: str,
    ephemeral_root: Any,
    alias_root: Any,
    memory_root: Any = None,
    main_seed_ram_max_bytes: Any = None,
) -> dict[str, Any]:
    root_info = _resolve_managed_worktree_root(
        ctx,
        ephemeral_root=ephemeral_root,
        memory_root=memory_root,
        main_seed_ram_max_bytes=main_seed_ram_max_bytes,
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
        "fallback_reason": normalize_optional_text(root_info.get("fallback_reason")),
        "default_line": normalize_optional_text(root_info.get("default_line")),
        "seed_snapshot_id": normalize_optional_text(root_info.get("seed_snapshot_id")),
        "seed_snapshot_total_bytes": root_info.get("seed_snapshot_total_bytes"),
        "main_seed_ram_max_bytes": root_info.get("main_seed_ram_max_bytes"),
    }


def resolve_task_auto_worktree_location(
    ctx: RepoContext,
    *,
    worktree_name: str,
    ephemeral_root: Any,
    alias_root: Any,
    memory_root: Any = None,
    main_seed_ram_max_bytes: Any = None,
) -> dict[str, Any]:
    return resolve_managed_worktree_location(
        ctx,
        worktree_name=worktree_name,
        ephemeral_root=ephemeral_root,
        alias_root=alias_root,
        memory_root=memory_root,
        main_seed_ram_max_bytes=main_seed_ram_max_bytes,
    )


def resolve_main_seed_mirror_location(
    ctx: RepoContext,
    *,
    seed_name: str,
    ephemeral_root: Any,
    memory_root: Any = None,
    main_seed_ram_max_bytes: Any = None,
) -> dict[str, Any] | None:
    root_info = _resolve_managed_worktree_root(
        ctx,
        ephemeral_root=ephemeral_root,
        memory_root=memory_root,
        main_seed_ram_max_bytes=main_seed_ram_max_bytes,
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
