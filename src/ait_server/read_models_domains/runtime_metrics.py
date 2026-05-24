from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Mapping

from ..server_paths import ServerContext


def _read_models_module():
    from .. import read_models as _read_models

    return _read_models


def _rounded_optional_float(value: Any, *, digits: int = 3) -> float | None:
    if value is None:
        return None
    try:
        return round(float(value), digits)
    except (TypeError, ValueError):
        return None


def normalize_live_turn_metrics(live_turn_metrics: Mapping[str, Any] | None) -> dict[str, Any]:
    metrics = dict(live_turn_metrics or {})
    summary = metrics.get("summary") if isinstance(metrics.get("summary"), Mapping) else {}
    sources = (metrics, summary)

    repo_counts: dict[str, int] = {}
    repo_mapping = _read_models_module()._first_present(sources, "active_repositories", "active_turns_by_repo")
    if isinstance(repo_mapping, Mapping):
        repo_counts = {
            str(repo_name): _read_models_module()._int_metric(active_turns)
            for repo_name, active_turns in repo_mapping.items()
            if str(repo_name).strip()
        }
    elif isinstance(metrics.get("repo_activity"), list):
        repo_counts = {
            str(row.get("repo_name") or ""): _read_models_module()._int_metric(row.get("active_turns"))
            for row in metrics.get("repo_activity") or []
            if isinstance(row, Mapping) and str(row.get("repo_name") or "").strip()
        }
    repo_counts = dict(sorted((repo_name, count) for repo_name, count in repo_counts.items() if count > 0))

    recent_completed_turns = [
        dict(row)
        for row in (metrics.get("recent_completed_turns") or [])
        if isinstance(row, Mapping)
    ]
    recent_failed_turns = [
        dict(row)
        for row in (metrics.get("recent_failed_turns") or [])
        if isinstance(row, Mapping)
    ]

    active_turns = _read_models_module()._int_metric(_read_models_module()._first_present(sources, "active_turns", "active_turn_count"))
    oldest_active_turn_started_at = _read_models_module()._first_present(
        sources,
        "oldest_active_turn_started_at",
        "oldest_active_turn_started_at_epoch_seconds",
    )
    oldest_active_turn_age_seconds = _rounded_optional_float(
        _read_models_module()._first_present(sources, "oldest_active_turn_age_seconds"),
    )
    recent_completed_p95_seconds = _rounded_optional_float(
        _read_models_module()._first_present(sources, "recent_completed_p95_seconds"),
    )
    recent_completed_turn_count = _read_models_module()._int_metric(
        _read_models_module()._first_present(sources, "recent_completed_turn_count"),
    ) or len(recent_completed_turns)
    recent_failed_turn_count = _read_models_module()._int_metric(
        _read_models_module()._first_present(sources, "recent_failed_turn_count"),
    ) or len(recent_failed_turns)

    normalized_summary = {
        "active_turns": active_turns,
        "active_repositories": len(repo_counts),
        "oldest_active_turn_started_at": oldest_active_turn_started_at,
        "oldest_active_turn_age_seconds": oldest_active_turn_age_seconds,
        "recent_completed_turns": recent_completed_turn_count,
        "recent_failed_turns": recent_failed_turn_count,
        "recent_completed_p95_seconds": recent_completed_p95_seconds,
    }
    repo_activity = [
        {"repo_name": repo_name, "active_turns": count}
        for repo_name, count in repo_counts.items()
    ]
    return {
        "summary": normalized_summary,
        "repo_activity": repo_activity,
        "active_turns": active_turns,
        "active_repositories": repo_counts,
        "oldest_active_turn_started_at": oldest_active_turn_started_at,
        "oldest_active_turn_age_seconds": oldest_active_turn_age_seconds,
        "recent_completed_turns": recent_completed_turns,
        "recent_failed_turns": recent_failed_turns,
        "recent_completed_p95_seconds": recent_completed_p95_seconds,
        "recent_completed_turn_count": recent_completed_turn_count,
        "recent_failed_turn_count": recent_failed_turn_count,
        "snapshot_at_epoch_seconds": _read_models_module()._first_present(sources, "snapshot_at_epoch_seconds"),
    }


def live_turn_pressure_summary(live_turn_metrics: Mapping[str, Any] | None) -> dict[str, Any]:
    normalized = normalize_live_turn_metrics(live_turn_metrics)
    summary = normalized["summary"]
    in_flight_turns = _read_models_module()._int_metric(summary.get("active_turns"))
    queued_turns = 0
    oldest_in_flight_turn_age_seconds = _rounded_optional_float(summary.get("oldest_active_turn_age_seconds"))
    oldest_queued_turn_age_seconds = None

    if in_flight_turns <= 0 and queued_turns <= 0:
        pressure_state = "idle"
    elif queued_turns > 0 or in_flight_turns >= 4 or (oldest_in_flight_turn_age_seconds or 0.0) >= 300.0:
        pressure_state = "saturated"
    elif in_flight_turns >= 2 or (oldest_in_flight_turn_age_seconds or 0.0) >= 120.0:
        pressure_state = "busy"
    else:
        pressure_state = "ok"

    return {
        "pressure_state": pressure_state,
        "in_flight_turns": in_flight_turns,
        "queued_turns": queued_turns,
        "active_repositories": _read_models_module()._int_metric(summary.get("active_repositories")),
        "active_repositories_by_name": dict(normalized.get("active_repositories") or {}),
        "oldest_in_flight_turn_started_at": summary.get("oldest_active_turn_started_at"),
        "oldest_in_flight_turn_age_seconds": oldest_in_flight_turn_age_seconds,
        "oldest_queued_turn_age_seconds": oldest_queued_turn_age_seconds,
        "recent_completed_turns": _read_models_module()._int_metric(summary.get("recent_completed_turns")),
        "recent_failed_turns": _read_models_module()._int_metric(summary.get("recent_failed_turns")),
        "recent_completed_p95_seconds": summary.get("recent_completed_p95_seconds"),
    }


def annotate_operator_read_payload(
    payload: Mapping[str, Any],
    *,
    cache_state: str = "computed",
    cache_age_seconds: float = 0.0,
    cache_ttl_seconds: float | None = None,
    cached_at: str | None = None,
) -> dict[str, Any]:
    ttl = _read_models_module().operator_pressure_cache_ttl_seconds() if cache_ttl_seconds is None else max(float(cache_ttl_seconds), 0.0)
    annotated = dict(payload)
    annotated["cache_state"] = str(cache_state or "computed")
    annotated["cache_age_seconds"] = round(max(float(cache_age_seconds), 0.0), 3)
    annotated["cache_ttl_seconds"] = ttl
    annotated["cached_at"] = str(cached_at or payload.get("snapshot_at") or _read_models_module().utc_now())
    return annotated


def _path_inventory(path: Path) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "path": str(path),
        "exists": path.exists(),
        "is_file": path.is_file(),
        "is_dir": path.is_dir(),
    }
    if path.is_file():
        try:
            payload["size_bytes"] = path.stat().st_size
        except OSError:
            payload["size_bytes"] = None
    return payload


def _telegram_inventory(path: Path) -> tuple[dict[str, Any], dict[str, int]]:
    base = _path_inventory(path)
    if not path.exists():
        base.update(
            {
                "last_update_id": 0,
                "chat_count": 0,
                "repo_names": [],
                "linked_session_count": 0,
                "linked_session_ids": [],
            }
        )
        return base, {}

    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        payload = {}
    chats = payload.get("chats") if isinstance(payload, Mapping) else {}
    if not isinstance(chats, Mapping):
        chats = {}

    repo_names: set[str] = set()
    session_ids: set[str] = set()
    chat_count_by_repo: dict[str, int] = {}
    for value in chats.values():
        if not isinstance(value, Mapping):
            continue
        repo_name = str(value.get("repo_name") or "").strip()
        session_id = str(value.get("session_id") or "").strip()
        if repo_name:
            repo_names.add(repo_name)
            chat_count_by_repo[repo_name] = chat_count_by_repo.get(repo_name, 0) + 1
        if session_id:
            session_ids.add(session_id)

    base.update(
        {
            "last_update_id": _read_models_module()._int_metric(payload.get("last_update_id") if isinstance(payload, Mapping) else 0),
            "chat_count": len(chats),
            "repo_names": sorted(repo_names),
            "linked_session_count": len(session_ids),
            "linked_session_ids": sorted(session_ids),
        }
    )
    return base, dict(sorted(chat_count_by_repo.items()))


def _workflow_session_inventory(ctx: ServerContext) -> dict[str, Any]:
    with _read_models_module().connect(ctx) as conn:
        session_rows = [
            dict(row)
            for row in conn.execute(
                "select repo_name, status from sessions order by updated_at desc, created_at desc"
            ).fetchall()
        ]
        planning_rows = [
            dict(row)
            for row in conn.execute(
                "select repo_name, status from planning_sessions order by updated_at desc, created_at desc"
            ).fetchall()
        ]

    per_repo: dict[str, dict[str, Any]] = {}
    for row in session_rows:
        repo_name = str(row.get("repo_name") or "")
        if not repo_name:
            continue
        slot = per_repo.setdefault(
            repo_name,
            {
                "repo_name": repo_name,
                "session_count": 0,
                "active_sessions": 0,
                "planning_session_count": 0,
                "active_planning_sessions": 0,
            },
        )
        slot["session_count"] += 1
        if str(row.get("status") or "") == "active":
            slot["active_sessions"] += 1

    for row in planning_rows:
        repo_name = str(row.get("repo_name") or "")
        if not repo_name:
            continue
        slot = per_repo.setdefault(
            repo_name,
            {
                "repo_name": repo_name,
                "session_count": 0,
                "active_sessions": 0,
                "planning_session_count": 0,
                "active_planning_sessions": 0,
            },
        )
        slot["planning_session_count"] += 1
        if str(row.get("status") or "") == "active":
            slot["active_planning_sessions"] += 1

    return {
        "total_sessions": len(session_rows),
        "active_sessions": sum(1 for row in session_rows if str(row.get("status") or "") == "active"),
        "planning_session_count": len(planning_rows),
        "active_planning_sessions": sum(1 for row in planning_rows if str(row.get("status") or "") == "active"),
        "repositories": sorted(per_repo.values(), key=lambda row: str(row.get("repo_name") or "")),
    }


__all__ = [
    "_rounded_optional_float",
    "normalize_live_turn_metrics",
    "live_turn_pressure_summary",
    "annotate_operator_read_payload",
    "_path_inventory",
    "_telegram_inventory",
    "_workflow_session_inventory",
]
