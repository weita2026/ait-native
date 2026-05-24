from __future__ import annotations

import asyncio
from http.client import RemoteDisconnected
import socket
import time
from typing import Any, Awaitable, Callable, Collection, Sequence
from urllib.error import URLError
from urllib.parse import urlparse

DEFAULT_RETRYABLE_ERRNOS = frozenset({54, 60, 61, 104, 110, 111})
DEFAULT_RETRYABLE_MARKERS = (
    "timed out",
    "connection reset by peer",
    "remote end closed connection without response",
    "temporarily unavailable",
    "connection aborted",
    "broken pipe",
    "network is unreachable",
)
DEFAULT_SERVER_READ_MARKERS = (
    "500 internal server error",
    "502 bad gateway",
    "503 service unavailable",
    "504 gateway timeout",
)


def timeout_value(timeout: float | None, *, minimum: float | None = None) -> float | None:
    if timeout is None:
        return minimum
    if minimum is None:
        return timeout
    return max(timeout, minimum)


def timeout_phrase(timeout: float | None) -> str:
    if timeout is None:
        return ""
    return f" after {timeout:g} seconds"


def exception_chain(exc: BaseException) -> list[BaseException]:
    chain: list[BaseException] = []
    seen: set[int] = set()
    current: BaseException | None = exc
    while current is not None and id(current) not in seen:
        chain.append(current)
        seen.add(id(current))
        if isinstance(current, URLError) and isinstance(current.reason, BaseException) and id(current.reason) not in seen:
            current = current.reason
            continue
        current = current.__cause__ or current.__context__
    return chain


def is_retryable_transport_error(
    exc: BaseException,
    *,
    errnos: Collection[int] = DEFAULT_RETRYABLE_ERRNOS,
    markers: Sequence[str] = DEFAULT_RETRYABLE_MARKERS,
) -> bool:
    for current in exception_chain(exc):
        if isinstance(
            current,
            (
                TimeoutError,
                socket.timeout,
                RemoteDisconnected,
                ConnectionResetError,
                BrokenPipeError,
                ConnectionAbortedError,
            ),
        ):
            return True
        if isinstance(current, OSError) and getattr(current, "errno", None) in errnos:
            return True
        text = str(current).strip().lower()
        if text and any(marker in text for marker in markers):
            return True
    return False


def is_retryable_server_read_error(
    exc: BaseException,
    *,
    errnos: Collection[int] = DEFAULT_RETRYABLE_ERRNOS,
    transport_markers: Sequence[str] = DEFAULT_RETRYABLE_MARKERS,
    server_markers: Sequence[str] = DEFAULT_SERVER_READ_MARKERS,
) -> bool:
    if is_retryable_transport_error(exc, errnos=errnos, markers=transport_markers):
        return True
    text = str(exc).strip().lower()
    return bool(text) and any(marker in text for marker in server_markers)


def retry_delay_seconds(base_delay_seconds: float, retry_index: int) -> float:
    return max(float(base_delay_seconds), 0.0) * float(2 ** max(int(retry_index), 0))


def retry_transport_operation(
    action: Callable[[], Any],
    *,
    attempts: int,
    base_delay_seconds: float,
    retry_filter: Callable[[BaseException], bool],
) -> Any:
    resolved_attempts = max(int(attempts), 1)
    for attempt in range(resolved_attempts):
        try:
            return action()
        except Exception as exc:
            if attempt + 1 >= resolved_attempts or not retry_filter(exc):
                raise
            time.sleep(retry_delay_seconds(base_delay_seconds, attempt))
    raise AssertionError("unreachable")


async def retry_transport_operation_async(
    action: Callable[[], Awaitable[Any]],
    *,
    attempts: int,
    base_delay_seconds: float,
    retry_filter: Callable[[BaseException], bool],
) -> Any:
    resolved_attempts = max(int(attempts), 1)
    for attempt in range(resolved_attempts):
        try:
            return await action()
        except Exception as exc:
            if attempt + 1 >= resolved_attempts or not retry_filter(exc):
                raise
            await asyncio.sleep(retry_delay_seconds(base_delay_seconds, attempt))
    raise AssertionError("unreachable")


def is_loopback_url(value: str) -> bool:
    try:
        parsed = urlparse(value)
    except ValueError:
        return False
    hostname = (parsed.hostname or "").strip().lower()
    return hostname in {"127.0.0.1", "localhost", "::1"}
