from __future__ import annotations

import fnmatch
import hashlib
import json
import os
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable, Iterable

from ait_protocol.runtime_roots import resolve_server_runtime_root
from .repo_paths import WORKTREE_CONFIG_NAME

IGNORED_DIRS = {".ait", ".ait-server", ".git", "__pycache__", ".pytest_cache", ".venv", "venv", ".mypy_cache"}
IGNORED_FILES = {".DS_Store", ".ait-worktree.json"}
WORKSPACE_IGNORE_FILE = ".aitignore"
WORKSPACE_DIGEST_CACHE_NAME = "digest_manifest_v1.json"
WORKSPACE_DIGEST_CACHE_VERSION = 1


@dataclass(frozen=True)
class WorkspaceIgnoreRule:
    source_text: str
    pattern: str
    negated: bool
    directory_only: bool
    anchored: bool
    basename_only: bool


def _elapsed_ms(start: float, end: float | None = None) -> float:
    finished = time.perf_counter() if end is None else end
    return round((finished - start) * 1000.0, 3)


def _workspace_visible_files(
    root: Path,
    *,
    ignore_rules: tuple[WorkspaceIgnoreRule, ...] | None = None,
    phase_timings_ms: dict[str, Any] | None = None,
) -> list[Path]:
    runtime_roots = _workspace_runtime_roots(root)
    workspace_rules = _load_workspace_ignore_rules(root) if ignore_rules is None else ignore_rules

    scan_started = time.perf_counter()
    candidates: list[Path] = []
    for dirpath, dirnames, filenames in os.walk(root, topdown=True, followlinks=False):
        dirnames[:] = [name for name in dirnames if name not in IGNORED_DIRS]
        base_path = Path(dirpath)
        candidates.extend(base_path / filename for filename in filenames if filename not in IGNORED_FILES)
    scan_finished = time.perf_counter()

    ignore_started = scan_finished
    visible_paths: list[Path] = []
    for path in candidates:
        rel = path.relative_to(root)
        if workspace_rules and _workspace_path_is_ignored(rel, workspace_rules):
            continue
        if runtime_roots:
            try:
                resolved_path = path.resolve()
            except (OSError, RuntimeError):
                resolved_path = path
            if any(resolved_path.is_relative_to(runtime_root) for runtime_root in runtime_roots):
                continue
        visible_paths.append(path)
    ignore_finished = time.perf_counter()

    if phase_timings_ms is not None:
        phase_timings_ms["workspace_scan"] = _elapsed_ms(scan_started, scan_finished)
        phase_timings_ms["ignore_filtering"] = _elapsed_ms(ignore_started, ignore_finished)
    return visible_paths


def _workspace_ignore_file(root: Path) -> Path:
    return root / WORKSPACE_IGNORE_FILE


def _parse_workspace_ignore_rule(line: str) -> WorkspaceIgnoreRule | None:
    text = line.strip()
    if not text or text.startswith("#"):
        return None
    escaped = False
    if text.startswith("\\#") or text.startswith("\\!"):
        text = text[1:]
        escaped = True
    negated = text.startswith("!") and not escaped
    if negated:
        text = text[1:]
    while text.startswith("./"):
        text = text[2:]
    anchored = text.startswith("/")
    if anchored:
        text = text[1:]
    directory_only = text.endswith("/")
    text = text.rstrip("/")
    if not text:
        return None
    return WorkspaceIgnoreRule(
        source_text=line.strip(),
        pattern=text,
        negated=negated,
        directory_only=directory_only,
        anchored=anchored,
        basename_only="/" not in text,
    )


def _load_workspace_ignore_rules(root: Path) -> tuple[WorkspaceIgnoreRule, ...]:
    path = _workspace_ignore_file(root)
    if not path.is_file():
        return ()
    return _parse_workspace_ignore_rules(path.read_text(encoding="utf-8", errors="replace"))


def _parse_workspace_ignore_rules(text: str) -> tuple[WorkspaceIgnoreRule, ...]:
    rules: list[WorkspaceIgnoreRule] = []
    for line in text.splitlines():
        rule = _parse_workspace_ignore_rule(line)
        if rule is not None:
            rules.append(rule)
    return tuple(rules)


def _workspace_ignore_rule_matches(rel_path: Path, rule: WorkspaceIgnoreRule) -> bool:
    parts = rel_path.parts
    if not parts:
        return False
    max_parts = len(parts) - 1 if rule.directory_only else len(parts)
    if max_parts <= 0:
        return False
    if rule.basename_only:
        return any(fnmatch.fnmatchcase(part, rule.pattern) for part in parts[:max_parts])
    starts = (0,) if rule.anchored else range(max_parts)
    for start in starts:
        for end in range(start + 1, max_parts + 1):
            candidate = "/".join(parts[start:end])
            if fnmatch.fnmatchcase(candidate, rule.pattern):
                return True
    return False


def _workspace_path_is_ignored(rel_path: Path, rules: tuple[WorkspaceIgnoreRule, ...]) -> bool:
    ignored = False
    for rule in rules:
        if _workspace_ignore_rule_matches(rel_path, rule):
            ignored = not rule.negated
    return ignored


def workspace_path_is_ignored(root: Path, path: Path | str) -> bool:
    rel_path = Path(path)
    if rel_path.is_absolute():
        rel_path = rel_path.relative_to(root.resolve())
    return _workspace_path_is_ignored(rel_path, _load_workspace_ignore_rules(root))


def _runtime_root_source(runtime_root: Path | None = None) -> str:
    if runtime_root is not None:
        return "explicit"
    if (os.environ.get("AIT_NATIVE_SERVER_DATA") or "").strip():
        return "AIT_NATIVE_SERVER_DATA"
    return "unconfigured"


def _resolved_server_runtime_root(runtime_root: Path | None = None) -> Path | None:
    if runtime_root is None:
        env_root = (os.environ.get("AIT_NATIVE_SERVER_DATA") or "").strip()
        if not env_root:
            return None
        return resolve_server_runtime_root()
    return Path(runtime_root).expanduser()


def _workspace_runtime_roots(root: Path, runtime_root: Path | None = None) -> tuple[Path, ...]:
    configured_runtime_root = _resolved_server_runtime_root(runtime_root)
    if configured_runtime_root is None:
        return ()
    try:
        resolved_root = root.resolve()
        resolved_runtime = configured_runtime_root.resolve()
        resolved_runtime.relative_to(resolved_root)
    except (OSError, RuntimeError, ValueError):
        return ()
    if resolved_runtime == resolved_root:
        return ()
    return (resolved_runtime,)


def workspace_runtime_root_hygiene(root: Path, runtime_root: Path | None = None) -> dict[str, Any]:
    """Return operator diagnostics for server runtime-root placement relative to a repo."""
    configured_runtime_root = _resolved_server_runtime_root(runtime_root)
    resolved_root = root.resolve()
    policy = _workspace_ignore_policy_for_rules(root, _load_workspace_ignore_rules(root))
    if configured_runtime_root is None:
        return {
            "repo_root": str(resolved_root),
            "runtime_root": None,
            "runtime_root_source": "unconfigured",
            "runtime_root_relative_to_repo": None,
            "inside_repo": False,
            "outside_repo": False,
            "equals_repo_root": False,
            "snapshot_ignored": False,
            "protected_from_snapshots": False,
            "state": "pass",
            "recommended_action": "configure_when_server_is_enabled",
            "reasons": ["No server runtime root is configured."],
            "next_actions": [
                "Configure AIT_NATIVE_SERVER_DATA only when this repository will run ait-server locally.",
            ],
            "ignore_policy": policy,
        }
    try:
        resolved_runtime = configured_runtime_root.resolve()
    except (OSError, RuntimeError):
        resolved_runtime = configured_runtime_root.absolute()

    try:
        runtime_rel = resolved_runtime.relative_to(resolved_root).as_posix()
        inside_repo = True
    except ValueError:
        runtime_rel = None
        inside_repo = False

    equals_repo_root = resolved_runtime == resolved_root
    ignored_runtime_roots = _workspace_runtime_roots(root, configured_runtime_root)
    snapshot_ignored = any(resolved_runtime == ignored for ignored in ignored_runtime_roots)
    protected_from_snapshots = (not inside_repo) or snapshot_ignored
    if equals_repo_root:
        state = "fail"
        recommended_action = "move_runtime_root_outside_repo"
        reasons = ["The configured runtime root is the repository checkout itself."]
        next_actions = [
            "Set AIT_NATIVE_SERVER_DATA to a dedicated directory outside the repo checkout.",
            "Move existing server data before creating new snapshots.",
        ]
    elif inside_repo:
        state = "warn"
        recommended_action = "prefer_external_runtime_root"
        reasons = ["The configured runtime root is inside the repository checkout but is ignored by snapshot/status scans."]
        next_actions = ["Prefer the default external runtime root or an absolute AIT_NATIVE_SERVER_DATA path outside the repo."]
    else:
        state = "pass"
        recommended_action = "none"
        reasons = ["The configured runtime root is outside the repository checkout."]
        next_actions = []

    return {
        "repo_root": str(resolved_root),
        "runtime_root": str(resolved_runtime),
        "runtime_root_source": _runtime_root_source(runtime_root),
        "runtime_root_relative_to_repo": runtime_rel,
        "inside_repo": inside_repo,
        "outside_repo": not inside_repo,
        "equals_repo_root": equals_repo_root,
        "snapshot_ignored": snapshot_ignored,
        "protected_from_snapshots": protected_from_snapshots,
        "state": state,
        "recommended_action": recommended_action,
        "reasons": reasons,
        "next_actions": next_actions,
        "ignore_policy": policy,
    }


def _workspace_ignore_policy_for_rules(root: Path, workspace_rules: tuple[WorkspaceIgnoreRule, ...]) -> dict[str, list[str]]:
    operational_roots = {".ait", ".ait-server"}
    runtime_roots: list[str] = []
    resolved_root = root.resolve()
    for runtime_root in _workspace_runtime_roots(root):
        try:
            runtime_rel = runtime_root.relative_to(resolved_root).as_posix()
        except ValueError:
            runtime_rel = str(runtime_root)
        runtime_roots.append(runtime_rel)
        operational_roots.add(runtime_rel)
    policy = {
        "dir_names": sorted(IGNORED_DIRS),
        "file_names": sorted(IGNORED_FILES),
        "operational_roots": sorted(operational_roots),
        "runtime_roots": sorted(runtime_roots),
    }
    if workspace_rules:
        policy["rule_files"] = [WORKSPACE_IGNORE_FILE]
        policy["custom_patterns"] = [rule.source_text for rule in workspace_rules]
    return policy


def workspace_ignore_policy(root: Path) -> dict[str, list[str]]:
    return _workspace_ignore_policy_for_rules(root, _load_workspace_ignore_rules(root))


def _workspace_digest_cache_path(root: Path) -> Path:
    workspace_dir = root / ".ait" / "workspace"
    if (root / WORKTREE_CONFIG_NAME).exists():
        try:
            resolved_root = root.resolve()
        except (OSError, RuntimeError):
            resolved_root = root.absolute()
        fingerprint = hashlib.sha256(str(resolved_root).encode("utf-8")).hexdigest()[:16]
        return workspace_dir / "worktrees" / fingerprint / WORKSPACE_DIGEST_CACHE_NAME
    return workspace_dir / WORKSPACE_DIGEST_CACHE_NAME


def _workspace_timestamp_ns(stat_result: os.stat_result, ns_attr: str, seconds_attr: str) -> int:
    value = getattr(stat_result, ns_attr, None)
    if value is not None:
        return int(value)
    return int(round(float(getattr(stat_result, seconds_attr)) * 1_000_000_000))


def _load_workspace_digest_cache(root: Path) -> dict[str, dict[str, Any]]:
    path = _workspace_digest_cache_path(root)
    if not path.is_file():
        return {}
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}
    if not isinstance(payload, dict) or int(payload.get("version") or 0) != WORKSPACE_DIGEST_CACHE_VERSION:
        return {}
    entries = payload.get("entries")
    if not isinstance(entries, dict):
        return {}
    normalized: dict[str, dict[str, Any]] = {}
    for rel, raw_entry in entries.items():
        if not isinstance(rel, str) or not isinstance(raw_entry, dict):
            continue
        sha256 = str(raw_entry.get("sha256") or "").strip()
        mode = str(raw_entry.get("mode") or "").strip()
        if not sha256 or not mode:
            continue
        try:
            normalized[rel] = {
                "sha256": sha256,
                "size_bytes": int(raw_entry.get("size_bytes") or 0),
                "mtime_ns": int(raw_entry.get("mtime_ns") or 0),
                "ctime_ns": int(raw_entry.get("ctime_ns") or 0),
                "mode": mode,
            }
        except (TypeError, ValueError):
            continue
    return normalized


def _store_workspace_digest_cache(root: Path, entries: dict[str, dict[str, Any]]) -> None:
    cache_path = _workspace_digest_cache_path(root)
    payload = {
        "version": WORKSPACE_DIGEST_CACHE_VERSION,
        "entries": {
            rel: {
                "sha256": str(entry.get("sha256") or ""),
                "size_bytes": int(entry.get("size_bytes") or 0),
                "mtime_ns": int(entry.get("mtime_ns") or 0),
                "ctime_ns": int(entry.get("ctime_ns") or 0),
                "mode": str(entry.get("mode") or ""),
            }
            for rel, entry in sorted(entries.items())
            if str(entry.get("sha256") or "").strip() and str(entry.get("mode") or "").strip()
        },
    }
    try:
        cache_path.parent.mkdir(parents=True, exist_ok=True)
        temp_path = cache_path.with_name(cache_path.name + ".tmp")
        temp_path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        temp_path.replace(cache_path)
    except OSError:
        return


def _workspace_digest_state(
    root: Path,
    path_pairs: Iterable[tuple[Path, str]],
    *,
    include_data: bool = False,
    phase_timings_ms: dict[str, Any] | None = None,
    prune_to_paths: bool = False,
) -> dict[str, dict[str, Any]]:
    state: dict[str, dict[str, Any]] = {}
    cached_entries = _load_workspace_digest_cache(root)
    updated_entries = dict(cached_entries)
    reused_path_count = 0
    rehashed_path_count = 0
    hashing_started = time.perf_counter()
    processed_paths: set[str] = set()
    for path, rel in sorted(path_pairs, key=lambda item: item[1]):
        stat_result = path.stat()
        mode = oct(stat_result.st_mode & 0o777)
        size_bytes = int(stat_result.st_size)
        mtime_ns = _workspace_timestamp_ns(stat_result, "st_mtime_ns", "st_mtime")
        ctime_ns = _workspace_timestamp_ns(stat_result, "st_ctime_ns", "st_ctime")
        cached = cached_entries.get(rel)
        cache_hit = (
            cached is not None
            and str(cached.get("sha256") or "").strip()
            and int(cached.get("size_bytes") or -1) == size_bytes
            and int(cached.get("mtime_ns") or -1) == mtime_ns
            and int(cached.get("ctime_ns") or -1) == ctime_ns
            and str(cached.get("mode") or "") == mode
        )
        data: bytes | None = None
        if cache_hit:
            sha256 = str(cached["sha256"])
            reused_path_count += 1
        else:
            data = path.read_bytes()
            sha256 = hashlib.sha256(data).hexdigest()
            rehashed_path_count += 1
        entry = {
            "path": rel,
            "sha256": sha256,
            "size_bytes": size_bytes,
            "mode": mode,
            "mtime_ns": mtime_ns,
            "ctime_ns": ctime_ns,
            "abs_path": path,
        }
        if include_data and data is not None:
            entry["data"] = data
        state[rel] = entry
        updated_entries[rel] = {
            "sha256": sha256,
            "size_bytes": size_bytes,
            "mtime_ns": mtime_ns,
            "ctime_ns": ctime_ns,
            "mode": mode,
        }
        processed_paths.add(rel)

    if prune_to_paths:
        updated_entries = {rel: updated_entries[rel] for rel in processed_paths if rel in updated_entries}

    _store_workspace_digest_cache(root, updated_entries)
    if phase_timings_ms is not None:
        phase_timings_ms["hashing"] = _elapsed_ms(hashing_started)
        phase_timings_ms["hashing_cache"] = {
            "reused_paths": reused_path_count,
            "rehashed_paths": rehashed_path_count,
        }
    return state


def _workspace_state(
    root: Path,
    *,
    ignore_rules: tuple[WorkspaceIgnoreRule, ...] | None = None,
    phase_timings_ms: dict[str, Any] | None = None,
) -> dict[str, dict[str, Any]]:
    visible_paths = _workspace_visible_files(root, ignore_rules=ignore_rules, phase_timings_ms=phase_timings_ms)
    return _workspace_digest_state(
        root,
        ((path, path.relative_to(root).as_posix()) for path in visible_paths),
        phase_timings_ms=phase_timings_ms,
        prune_to_paths=True,
    )


def _snapshot_workspace_ignore_rules(
    snapshot_id: str | None,
    snapshot_files: dict[str, dict[str, Any]],
    read_blob_text: Callable[[str], str],
) -> tuple[WorkspaceIgnoreRule, ...]:
    if snapshot_id is None:
        return ()
    ignore_entry = snapshot_files.get(WORKSPACE_IGNORE_FILE)
    if ignore_entry is None:
        return ()
    return _parse_workspace_ignore_rules(read_blob_text(str(ignore_entry["blob_id"])))


def _normalize_workspace_restore_path(path: str | Path) -> str:
    text = str(path).replace("\\", "/").strip()
    if text in {"", "."} or text.startswith("/"):
        raise ValueError(f"Restore path must be workspace-relative: {path}")
    rel = Path(text).as_posix()
    if rel in {"", "."} or rel.startswith("../") or "/../" in rel:
        raise ValueError(f"Restore path must stay within the workspace: {path}")
    return rel


def iter_workspace_files(root: Path, *, ignore_rules: tuple[WorkspaceIgnoreRule, ...] | None = None) -> Iterable[Path]:
    for path in _workspace_visible_files(root, ignore_rules=ignore_rules):
        yield path
