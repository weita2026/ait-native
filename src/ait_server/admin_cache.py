from __future__ import annotations

import os
import threading
import time
from typing import Any, Callable

from .agent_transport_runtime import utc_now_iso

_ADMIN_RESPONSE_CACHE: dict[tuple[str, int, int], dict[str, Any]] = {}
_ADMIN_RESPONSE_CACHE_LOCK = threading.Lock()


def _admin_metrics_cache_ttl_seconds() -> float:
    raw = str(os.environ.get("AIT_SERVER_PRESSURE_METRICS_CACHE_TTL_SECONDS", "5")).strip()
    try:
        return max(float(raw), 0.0)
    except ValueError:
        return 5.0


def _clear_admin_response_cache() -> None:
    with _ADMIN_RESPONSE_CACHE_LOCK:
        _ADMIN_RESPONSE_CACHE.clear()


def _annotated_admin_payload(
    payload: dict[str, Any],
    *,
    cache_state: str,
    cache_age_seconds: float,
    cache_ttl_seconds: float,
    cached_at: str,
) -> dict[str, Any]:
    annotated = dict(payload)
    annotated["cache_state"] = cache_state
    annotated["cache_age_seconds"] = round(max(cache_age_seconds, 0.0), 3)
    annotated["cache_ttl_seconds"] = cache_ttl_seconds
    annotated["cached_at"] = cached_at
    return annotated


def _cached_admin_payload(name: str, key: tuple[int, int], compute: Callable[[], dict[str, Any]]) -> dict[str, Any]:
    ttl = _admin_metrics_cache_ttl_seconds()
    cache_key = (name, *key)
    now = time.monotonic()
    if ttl > 0:
        with _ADMIN_RESPONSE_CACHE_LOCK:
            entry = _ADMIN_RESPONSE_CACHE.get(cache_key)
        if entry is not None:
            age = now - float(entry["stored_at_monotonic"])
            if age <= ttl:
                return _annotated_admin_payload(
                    dict(entry["payload"]),
                    cache_state="cached",
                    cache_age_seconds=age,
                    cache_ttl_seconds=ttl,
                    cached_at=str(entry["cached_at"]),
                )
    payload = dict(compute())
    cached_at = utc_now_iso()
    annotated = _annotated_admin_payload(
        payload,
        cache_state="computed",
        cache_age_seconds=0.0,
        cache_ttl_seconds=ttl,
        cached_at=cached_at,
    )
    if ttl > 0:
        with _ADMIN_RESPONSE_CACHE_LOCK:
            _ADMIN_RESPONSE_CACHE[cache_key] = {
                "payload": dict(payload),
                "stored_at_monotonic": now,
                "cached_at": cached_at,
            }
    return annotated
