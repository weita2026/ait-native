from __future__ import annotations

import json
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

APP_DIR = ".ait"
WORKTREE_CONFIG_NAME = ".ait-worktree.json"
CONFIG_NAME = "config.json"
CONTENT_DB_NAME = "content.db"
CONTROL_DB_NAME = "control.db"
REPO_DISCOVERY_ENV_VARS = ("AIT_REPO_ROOT", "AIT_NATIVE_WORKSPACE_ROOT", "AIT_WORKSPACE_ROOT")


def configured_repo_discovery_start() -> Path | None:
    for name in REPO_DISCOVERY_ENV_VARS:
        raw = os.environ.get(name)
        if raw and raw.strip():
            return Path(raw.strip()).expanduser()
    return None


@dataclass(frozen=True)
class RepoContext:
    root: Path
    ait_dir: Path
    content_db_path: Path
    control_db_path: Path
    config_path: Path
    worktree_config_path: Path | None = None

    @property
    def manifest_dir(self) -> Path:
        return self.ait_dir / "objects" / "manifests"

    @property
    def pack_dir(self) -> Path:
        return self.ait_dir / "objects" / "packs"

    @property
    def tree_pack_dir(self) -> Path:
        return self.ait_dir / "objects" / "tree-packs"

    @property
    def ref_dir(self) -> Path:
        return self.ait_dir / "refs" / "lines"

    @property
    def workspace_dir(self) -> Path:
        return self.ait_dir / "workspace"

    @property
    def worktree_registry_dir(self) -> Path:
        return self.ait_dir / "worktrees"

    @property
    def policy_path(self) -> Path:
        return self.ait_dir / "policy.yaml"

    @property
    def repo_root(self) -> Path:
        return self.ait_dir.resolve().parent

    @property
    def is_worktree(self) -> bool:
        return self.worktree_config_path is not None

    @classmethod
    def _discover_from(cls, start: Path) -> "RepoContext":
        start = start.resolve()
        cur = start
        while True:
            ait_dir = cur / APP_DIR
            worktree_config_path = cur / WORKTREE_CONFIG_NAME
            if ait_dir.is_dir() and worktree_config_path.exists():
                return cls(
                    root=cur,
                    ait_dir=ait_dir,
                    content_db_path=ait_dir / CONTENT_DB_NAME,
                    control_db_path=ait_dir / CONTROL_DB_NAME,
                    config_path=ait_dir / CONFIG_NAME,
                    worktree_config_path=worktree_config_path,
                )
            if ait_dir.is_dir():
                return cls(
                    root=cur,
                    ait_dir=ait_dir,
                    content_db_path=ait_dir / CONTENT_DB_NAME,
                    control_db_path=ait_dir / CONTROL_DB_NAME,
                    config_path=ait_dir / CONFIG_NAME,
                    worktree_config_path=None,
                )
            if cur.parent == cur:
                raise FileNotFoundError("No .ait directory found in current path or parents.")
            cur = cur.parent

    @classmethod
    def discover(cls, start: Optional[Path] = None) -> "RepoContext":
        if start is not None:
            return cls._discover_from(start.expanduser())
        try:
            return cls._discover_from(Path.cwd())
        except FileNotFoundError:
            configured = configured_repo_discovery_start()
            if configured is None:
                raise
            return cls._discover_from(configured)


def _read_json_object(path: Path | None) -> dict[str, object]:
    if path is None or not path.exists():
        return {}
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    return payload if isinstance(payload, dict) else {}


def configured_repo_name(ctx: RepoContext) -> str | None:
    base = _read_json_object(ctx.config_path)
    overlay = _read_json_object(ctx.worktree_config_path)
    value = overlay.get("repo_name")
    if value is None:
        value = base.get("repo_name")
    text = str(value or "").strip()
    return text or None


def _normalize_bound_root_candidate(value: Path | str | None) -> Path | None:
    if value is None:
        return None
    text = str(value).strip()
    if not text:
        return None
    return Path(text).expanduser()


def _validated_bound_root(candidate: Path | str | None, *, repo_name: str | None = None) -> Path | None:
    resolved_candidate = _normalize_bound_root_candidate(candidate)
    if resolved_candidate is None:
        return None
    try:
        ctx = RepoContext.discover(resolved_candidate)
    except FileNotFoundError:
        return None
    actual_repo_name = configured_repo_name(ctx) or ctx.root.name
    if repo_name and actual_repo_name != repo_name:
        return None
    return ctx.root.resolve()


def _nearby_repo_checkout(repo_name: str | None, fallback_root: Path) -> Path | None:
    normalized_repo_name = str(repo_name or "").strip()
    if not normalized_repo_name:
        return None
    search_roots: list[Path] = []
    seen_roots: set[Path] = set()
    for base in (fallback_root, Path.cwd()):
        try:
            resolved = base.expanduser().resolve()
        except OSError:
            resolved = base.expanduser()
        for parent in (resolved.parent, *list(resolved.parents[:2])):
            if parent in seen_roots:
                continue
            seen_roots.add(parent)
            search_roots.append(parent)
    for parent in search_roots:
        direct = _validated_bound_root(parent / normalized_repo_name, repo_name=normalized_repo_name)
        if direct is not None:
            return direct
        try:
            children = sorted(parent.iterdir(), key=lambda item: item.name)
        except OSError:
            continue
        for child in children:
            if not child.is_dir():
                continue
            matched = _validated_bound_root(child, repo_name=normalized_repo_name)
            if matched is not None:
                return matched
    return None


def resolve_bound_repo_root(
    repo_name: str | None,
    *,
    preferred_workspace_root: Path | str | None = None,
    preferred_repo_root: Path | str | None = None,
    fallback_root: Path | str | None = None,
) -> Path:
    expected_repo_name = str(repo_name or "").strip() or None
    configured_start = configured_repo_discovery_start()
    base_fallback = Path(fallback_root).expanduser() if fallback_root is not None else (
        configured_start if configured_start is not None else Path.cwd()
    )
    for candidate in (preferred_workspace_root, preferred_repo_root):
        matched = _validated_bound_root(candidate, repo_name=expected_repo_name)
        if matched is not None:
            return matched
    matched_fallback = _validated_bound_root(base_fallback, repo_name=expected_repo_name)
    if matched_fallback is not None:
        return matched_fallback
    nearby = _nearby_repo_checkout(expected_repo_name, base_fallback)
    if nearby is not None:
        return nearby
    return base_fallback.resolve()
