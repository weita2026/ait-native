from __future__ import annotations

import contextvars
import json
import shlex
import time
from pathlib import Path
from typing import Any

import typer

from ait_protocol.common import utc_now

from .repo_paths import RepoContext
from .store_repo_config import load_config

_ACTIVE_COMMAND_PROFILE: contextvars.ContextVar[dict[str, Any] | None] = contextvars.ContextVar(
    "ait_active_command_profile",
    default=None,
)


def _normalize_text_value(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _normalize_command_profiling_mode(value: Any) -> str | None:
    text = _normalize_text_value(value)
    if text is None:
        return None
    lowered = text.lower()
    if lowered in {"on", "true", "yes"}:
        return "on"
    if lowered in {"off", "false", "no"}:
        return "off"
    raise typer.BadParameter("`--command-profiling` must be `on` or `off`.")


def _command_profiling_mode(ctx: RepoContext | None) -> str:
    if ctx is None:
        return "off"
    try:
        return _normalize_command_profiling_mode(load_config(ctx).get("command_profiling")) or "off"
    except typer.BadParameter:
        return "off"


def _command_profiling_enabled(ctx: RepoContext | None) -> bool:
    return _command_profiling_mode(ctx) == "on"


def _command_profiling_artifact_root(ctx: RepoContext) -> Path:
    return ctx.ait_dir / "generated" / "profiling"


def _command_profiling_log_path(ctx: RepoContext) -> Path:
    return _command_profiling_artifact_root(ctx) / "commands.jsonl"


def _command_profile_elapsed_ms(started_ns: int) -> float:
    return round(max((time.perf_counter_ns() - int(started_ns)) / 1_000_000, 0.0), 3)


def _command_profile_raw_command(command_tokens: list[str]) -> str:
    if not command_tokens:
        return "ait"
    return "ait " + " ".join(shlex.quote(str(token)) for token in command_tokens)


def _command_profile_tracker(ctx: RepoContext, command_tokens: list[str]) -> dict[str, Any]:
    return {
        "ctx": ctx,
        "argv": [str(token) for token in command_tokens],
        "command": _command_profile_raw_command(command_tokens),
        "started_at": utc_now(),
        "started_ns": time.perf_counter_ns(),
        "phase_timings_ms": {},
    }


def _start_command_profiling(command_tokens: list[str]) -> dict[str, Any] | None:
    if not command_tokens:
        return None
    try:
        ctx = RepoContext.discover()
    except FileNotFoundError:
        return None
    if not _command_profiling_enabled(ctx):
        return None
    tracker = _command_profile_tracker(ctx, command_tokens)
    tracker["_context_token"] = _ACTIVE_COMMAND_PROFILE.set(tracker)
    return tracker


def _command_profile_record_phase(tracker: dict[str, Any] | None, name: str, value: Any) -> None:
    if tracker is None:
        tracker = _ACTIVE_COMMAND_PROFILE.get()
    if not tracker:
        return
    phase_name = str(name or "").strip()
    if not phase_name:
        return
    tracker.setdefault("phase_timings_ms", {})[phase_name] = value


def _command_profile_payload(ctx: RepoContext, tracker: dict[str, Any], returncode: int) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "command": str(tracker["command"]),
        "argv": list(tracker["argv"]),
        "started_at": str(tracker["started_at"]),
        "finished_at": utc_now(),
        "duration_ms": _command_profile_elapsed_ms(int(tracker["started_ns"])),
        "returncode": int(returncode),
        "success": int(returncode) == 0,
        "repo_root": str(ctx.repo_root),
        "workspace_root": str(ctx.root),
    }
    cfg = load_config(ctx)
    worktree_name = _normalize_text_value(cfg.get("worktree_name"))
    if worktree_name is not None:
        payload["worktree_name"] = worktree_name
    try:
        from .store import current_line

        payload["line_name"] = current_line(ctx)
    except Exception:
        payload["line_name"] = None
    phase_timings = tracker.get("phase_timings_ms")
    if isinstance(phase_timings, dict) and phase_timings:
        payload["phase_timings_ms"] = phase_timings
    return payload


def _write_command_profile_row(ctx: RepoContext, payload: dict[str, Any]) -> None:
    log_path = _command_profiling_log_path(ctx)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(payload, ensure_ascii=False, separators=(",", ":")) + "\n")


def _finish_command_profiling(tracker: dict[str, Any] | None, returncode: int) -> None:
    if not tracker:
        return
    ctx = tracker["ctx"]
    token = tracker.pop("_context_token", None)
    try:
        _write_command_profile_row(ctx, _command_profile_payload(ctx, tracker, int(returncode)))
    except OSError:
        pass
    finally:
        if token is not None:
            _ACTIVE_COMMAND_PROFILE.reset(token)
