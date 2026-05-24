from __future__ import annotations

import threading
import time
from collections import Counter, deque
from dataclasses import dataclass
from math import ceil
from typing import Any, Callable, Mapping
from uuid import uuid4


@dataclass(slots=True)
class _ActiveTurn:
    token: str
    repo_name: str
    session_id: str | None
    surface: str | None
    started_at_epoch_seconds: float
    metadata: dict[str, Any]


class LiveTurnRegistry:
    def __init__(
        self,
        *,
        time_fn: Callable[[], float] | None = None,
        recent_completed_limit: int = 20,
    ) -> None:
        limit = int(recent_completed_limit)
        if limit <= 0:
            raise ValueError("recent_completed_limit must be greater than zero")
        self._time_fn = time_fn or time.time
        self._recent_completed_limit = limit
        self._lock = threading.Lock()
        self._active_turns: dict[str, _ActiveTurn] = {}
        self._recent_finished_turns: deque[dict[str, Any]] = deque(maxlen=limit)

    def start(
        self,
        *,
        repo_name: str,
        session_id: str | None = None,
        surface: str | None = None,
        metadata: Mapping[str, Any] | None = None,
        **extra_metadata: Any,
    ) -> str:
        normalized_repo_name = str(repo_name or "").strip()
        if not normalized_repo_name:
            raise ValueError("repo_name is required")
        merged_metadata = dict(metadata or {})
        merged_metadata.update(extra_metadata)
        with self._lock:
            token = uuid4().hex
            while token in self._active_turns:
                token = uuid4().hex
            self._active_turns[token] = _ActiveTurn(
                token=token,
                repo_name=normalized_repo_name,
                session_id=_normalize_optional_text(session_id),
                surface=_normalize_optional_text(surface),
                started_at_epoch_seconds=float(self._time_fn()),
                metadata=merged_metadata,
            )
        return token

    def finish(self, token: str, **metadata: Any) -> dict[str, Any]:
        normalized_token = str(token or "").strip()
        if not normalized_token:
            return {}
        with self._lock:
            active_turn = self._active_turns.pop(normalized_token, None)
            if active_turn is None:
                return {}
            finished_at_epoch_seconds = float(self._time_fn())
            raw_completion_metadata = dict(metadata)
            outcome = _completion_outcome(raw_completion_metadata)
            failed = _completion_failed(outcome=outcome, completion_metadata=raw_completion_metadata)
            completion_metadata = _completion_metadata_payload(raw_completion_metadata)
            completed_turn = {
                "turn_token": active_turn.token,
                "repo_name": active_turn.repo_name,
                "session_id": active_turn.session_id,
                "surface": active_turn.surface,
                "started_at_epoch_seconds": active_turn.started_at_epoch_seconds,
                "finished_at_epoch_seconds": finished_at_epoch_seconds,
                "duration_seconds": max(0.0, finished_at_epoch_seconds - active_turn.started_at_epoch_seconds),
                "outcome": outcome,
                "failed": failed,
                "metadata": dict(active_turn.metadata),
                "completion_metadata": completion_metadata,
            }
            error = _normalize_optional_text(raw_completion_metadata.get("error"))
            if error is not None:
                completed_turn["error"] = error
            self._recent_finished_turns.append(completed_turn)
        return dict(completed_turn)

    def snapshot(self, *, now: float | None = None, recent_completed_limit: int | None = None) -> dict[str, Any]:
        if recent_completed_limit is not None and int(recent_completed_limit) < 0:
            raise ValueError("recent_completed_limit must be greater than or equal to zero")
        snapshot_at = float(self._time_fn()) if now is None else float(now)
        with self._lock:
            active_turns = list(self._active_turns.values())
            recent_finished_turns = list(self._recent_finished_turns)

        active_turn_count = len(active_turns)
        oldest_started_at = min((turn.started_at_epoch_seconds for turn in active_turns), default=None)
        active_turns_by_repo = dict(sorted(Counter(turn.repo_name for turn in active_turns).items()))

        recent_finished = list(reversed(recent_finished_turns))
        if recent_completed_limit is not None:
            recent_finished = recent_finished[: int(recent_completed_limit)]
        recent_completed = [dict(item) for item in recent_finished if not bool(item.get("failed"))]
        recent_failed = [dict(item) for item in recent_finished if bool(item.get("failed"))]
        recent_completed_p95_seconds = _p95_seconds(
            [float(item.get("duration_seconds") or 0.0) for item in recent_completed],
        )

        return {
            "active_turns": active_turn_count,
            "active_repositories": active_turns_by_repo,
            "oldest_active_turn_started_at": oldest_started_at,
            "oldest_active_turn_age_seconds": (
                max(0.0, snapshot_at - oldest_started_at) if oldest_started_at is not None else None
            ),
            "recent_completed_turns": recent_completed,
            "recent_failed_turns": recent_failed,
            "recent_completed_p95_seconds": recent_completed_p95_seconds,
            "snapshot_at_epoch_seconds": snapshot_at,
            "active_turn_count": active_turn_count,
            "oldest_active_turn_started_at_epoch_seconds": oldest_started_at,
            "active_turns_by_repo": active_turns_by_repo,
            "recent_completed_turn_count": len(recent_completed),
            "recent_failed_turn_count": len(recent_failed),
        }

    def reset_for_tests(self) -> None:
        with self._lock:
            self._active_turns.clear()
            self._recent_finished_turns.clear()

    def snapshot_live_turn_metrics(self) -> dict[str, Any]:
        return self.snapshot()


def _normalize_optional_text(value: Any) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _completion_outcome(completion_metadata: Mapping[str, Any]) -> str:
    for key in ("outcome", "status", "result"):
        value = _normalize_optional_text(completion_metadata.get(key))
        if value is not None:
            return value
    if "ok" in completion_metadata and isinstance(completion_metadata.get("ok"), bool):
        return "ok" if bool(completion_metadata.get("ok")) else "failed"
    if _normalize_optional_text(completion_metadata.get("error")):
        return "failed"
    return "completed"


def _completion_failed(*, outcome: str, completion_metadata: Mapping[str, Any]) -> bool:
    failed_value = completion_metadata.get("failed")
    if isinstance(failed_value, bool):
        return failed_value
    ok_value = completion_metadata.get("ok")
    if isinstance(ok_value, bool):
        return not ok_value
    if _normalize_optional_text(completion_metadata.get("error")):
        return True
    lowered = outcome.strip().lower()
    return lowered in {"error", "failed", "failure"}


def _completion_metadata_payload(completion_metadata: Mapping[str, Any]) -> dict[str, Any]:
    payload = dict(_mapping_payload(completion_metadata.get("metadata")))
    for key, value in completion_metadata.items():
        if key in {"outcome", "status", "result", "ok", "failed", "error", "metadata"}:
            continue
        payload[key] = value
    return payload


def _mapping_payload(value: Any) -> dict[str, Any]:
    if isinstance(value, Mapping):
        return dict(value)
    return {}


def _p95_seconds(values: list[float]) -> float | None:
    if not values:
        return None
    ordered = sorted(float(value) for value in values)
    index = max(0, ceil(len(ordered) * 0.95) - 1)
    return ordered[index]


_DEFAULT_REGISTRY = LiveTurnRegistry()


def get_live_turn_registry() -> LiveTurnRegistry:
    return _DEFAULT_REGISTRY


def start_live_turn(
    *,
    repo_name: str,
    session_id: str | None = None,
    surface: str | None = None,
    metadata: Mapping[str, Any] | None = None,
    **extra_metadata: Any,
) -> str:
    return _DEFAULT_REGISTRY.start(
        repo_name=repo_name,
        session_id=session_id,
        surface=surface,
        metadata=metadata,
        **extra_metadata,
    )


def finish_live_turn(
    token: str,
    **metadata: Any,
) -> dict[str, Any]:
    return _DEFAULT_REGISTRY.finish(token, **metadata)


def snapshot_live_turn_metrics() -> dict[str, Any]:
    return _DEFAULT_REGISTRY.snapshot_live_turn_metrics()


def export_live_turns_snapshot(*, now: float | None = None, recent_completed_limit: int | None = None) -> dict[str, Any]:
    return _DEFAULT_REGISTRY.snapshot(now=now, recent_completed_limit=recent_completed_limit)


def reset_live_turns_for_tests() -> None:
    _DEFAULT_REGISTRY.reset_for_tests()


__all__ = [
    "LiveTurnRegistry",
    "export_live_turns_snapshot",
    "finish_live_turn",
    "get_live_turn_registry",
    "reset_live_turns_for_tests",
    "snapshot_live_turn_metrics",
    "start_live_turn",
]
