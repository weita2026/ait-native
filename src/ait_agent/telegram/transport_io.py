from __future__ import annotations

import json
import os
import socket
import uuid
from http.client import RemoteDisconnected
from pathlib import Path
from typing import Any, Mapping
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

import httpx

from ait_agent.transport_retry import (
    exception_chain as _shared_exception_chain,
    timeout_phrase as _shared_timeout_phrase,
)

from .config import BotRuntimeError
from .worker_config import AIT_TELEGRAM_TERMINATION_CONTEXT_ENV


def _clean_optional_str(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _compact_mapping(payload: Mapping[str, Any]) -> dict[str, Any]:
    compact: dict[str, Any] = {}
    for key, value in payload.items():
        if value is None:
            continue
        if isinstance(value, Mapping):
            nested = _compact_mapping(value)
            if nested:
                compact[key] = nested
            continue
        if isinstance(value, list) and not value:
            continue
        compact[key] = value
    return compact


def _runtime_backend_signature(mode: object, remote_name: object = None, server_url: object = None) -> str:
    return "|".join(
        (
            _clean_optional_str(mode) or "unknown",
            _clean_optional_str(remote_name) or "-",
            _clean_optional_str(server_url) or "-",
        )
    )


def _runtime_link_fields(runtime_snapshot: object | None) -> dict[str, Any]:
    if runtime_snapshot is None:
        return {}
    return {
        "runtime_backend_mode": getattr(runtime_snapshot, "mode", None),
        "runtime_backend_remote_name": getattr(runtime_snapshot, "remote_name", None),
        "runtime_backend_server_url": getattr(runtime_snapshot, "server_url", None),
        "runtime_backend_signature": getattr(runtime_snapshot, "signature", None),
    }


def _runtime_signature_from_link(link: Mapping[str, Any] | None) -> str | None:
    if not isinstance(link, Mapping):
        return None
    direct = _clean_optional_str(link.get("runtime_backend_signature"))
    if direct is not None:
        return direct
    mode = _clean_optional_str(link.get("runtime_backend_mode"))
    if mode is None:
        return None
    return _runtime_backend_signature(
        mode,
        link.get("runtime_backend_remote_name"),
        link.get("runtime_backend_server_url"),
    )


def _positive_int(value: object) -> int | None:
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    return parsed if parsed > 0 else None


def _termination_context_path(env: Mapping[str, str] | None = None) -> Path | None:
    source_env = os.environ if env is None else env
    raw = str(source_env.get(AIT_TELEGRAM_TERMINATION_CONTEXT_ENV) or "").strip()
    if not raw:
        return None
    return Path(raw).expanduser()


def _consume_pending_termination_context(
    *,
    pid: int | None = None,
    env: Mapping[str, str] | None = None,
) -> dict[str, Any] | None:
    path = _termination_context_path(env)
    if path is None or not path.exists():
        return None
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return None
    if not isinstance(payload, dict):
        return None
    expected_pid = os.getpid() if pid is None else pid
    try:
        context_pid = int(payload.get("pid") or 0)
    except (TypeError, ValueError):
        return None
    if context_pid != expected_pid:
        return None
    try:
        path.unlink(missing_ok=True)
    except OSError:
        pass
    return payload


def _signal_stop_suffix(signum: int) -> str:
    payload = _consume_pending_termination_context()
    if not payload:
        return ""
    details: list[str] = [f"signal={signum}"]
    reason = str(payload.get("reason") or "").strip()
    if reason:
        details.append(f"reason={reason}")
    worker_name = str(payload.get("worker_name") or "").strip()
    if worker_name:
        details.append(f"worker={worker_name}")
    issued_at = str(payload.get("issued_at") or "").strip()
    if issued_at:
        details.append(f"issued_at={issued_at}")
    issued_by_pid = payload.get("issued_by_pid")
    if issued_by_pid is not None:
        details.append(f"issued_by_pid={issued_by_pid}")
    return f" ({', '.join(details)})"


def _parse_int(value: str | None, fallback: int, minimum: int) -> int:
    try:
        parsed = int(value or "")
    except ValueError:
        return fallback
    if parsed <= 0:
        return fallback
    return max(parsed, minimum)


def _parse_bool(value: str | None, default: bool) -> bool:
    raw = str(value or "").strip().lower()
    if not raw:
        return default
    if raw in {"1", "true", "yes", "on"}:
        return True
    if raw in {"0", "false", "no", "off"}:
        return False
    return default


def _json_request(
    url: str,
    *,
    method: str = "GET",
    payload: dict[str, Any] | None = None,
    headers: Mapping[str, str] | None = None,
    timeout: float | None = 20.0,
    request_cls: type[Request] = Request,
    urlopen_impl=urlopen,
) -> Any:
    request_headers = {"Accept": "application/json"}
    if payload is not None:
        request_headers["Content-Type"] = "application/json"
    if headers:
        request_headers.update({str(key): str(value) for key, value in headers.items()})
    body = None if payload is None else json.dumps(payload, ensure_ascii=False).encode("utf-8")
    request = request_cls(url, data=body, headers=request_headers, method=method.upper())
    try:
        with urlopen_impl(request, timeout=timeout) as response:
            raw = response.read().decode("utf-8")
    except (TimeoutError, socket.timeout) as exc:
        raise BotRuntimeError(f"{method.upper()} {url} timed out{_shared_timeout_phrase(timeout)}.") from exc
    except OverflowError as exc:
        raise BotRuntimeError(f"{method.upper()} {url} failed: invalid timeout value {timeout!r}.") from exc
    except HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace") if exc.fp is not None else ""
        raise BotRuntimeError(f"{method.upper()} {url} failed: {exc.code} {detail or exc.reason}") from exc
    except URLError as exc:
        raise BotRuntimeError(f"{method.upper()} {url} failed: {exc.reason}") from exc
    except (RemoteDisconnected, ConnectionResetError, BrokenPipeError, ConnectionAbortedError, OSError) as exc:
        raise BotRuntimeError(f"{method.upper()} {url} failed: {exc}") from exc
    if not raw.strip():
        return {}
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return raw


def _multipart_json_request(
    url: str,
    *,
    fields: Mapping[str, object],
    file_field: str,
    file_name: str,
    file_bytes: bytes,
    mime_type: str,
    headers: Mapping[str, str] | None = None,
    timeout: float | None = 20.0,
    request_cls: type[Request] = Request,
    urlopen_impl=urlopen,
) -> Any:
    boundary = f"aittelegram-{uuid.uuid4().hex}"
    chunks: list[bytes] = []
    for key, value in fields.items():
        if value is None:
            continue
        chunks.extend(
            [
                f"--{boundary}\r\n".encode("utf-8"),
                f'Content-Disposition: form-data; name="{key}"\r\n\r\n'.encode("utf-8"),
                str(value).encode("utf-8"),
                b"\r\n",
            ]
        )
    chunks.extend(
        [
            f"--{boundary}\r\n".encode("utf-8"),
            (
                f'Content-Disposition: form-data; name="{file_field}"; filename="{file_name}"\r\n'
                f"Content-Type: {mime_type}\r\n\r\n"
            ).encode("utf-8"),
            file_bytes,
            b"\r\n",
            f"--{boundary}--\r\n".encode("utf-8"),
        ]
    )
    request_headers = {
        "Accept": "application/json",
        "Content-Type": f"multipart/form-data; boundary={boundary}",
    }
    if headers:
        request_headers.update({str(key): str(value) for key, value in headers.items()})
    request = request_cls(url, data=b"".join(chunks), headers=request_headers, method="POST")
    try:
        with urlopen_impl(request, timeout=timeout) as response:
            raw = response.read().decode("utf-8")
    except (TimeoutError, socket.timeout) as exc:
        raise BotRuntimeError(f"POST {url} timed out{_shared_timeout_phrase(timeout)}.") from exc
    except OverflowError as exc:
        raise BotRuntimeError(f"POST {url} failed: invalid timeout value {timeout!r}.") from exc
    except HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace") if exc.fp is not None else ""
        raise BotRuntimeError(f"POST {url} failed: {exc.code} {detail or exc.reason}") from exc
    except URLError as exc:
        raise BotRuntimeError(f"POST {url} failed: {exc.reason}") from exc
    except (RemoteDisconnected, ConnectionResetError, BrokenPipeError, ConnectionAbortedError, OSError) as exc:
        raise BotRuntimeError(f"POST {url} failed: {exc}") from exc
    if not raw.strip():
        return {}
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return raw


def _bytes_request(
    url: str,
    *,
    method: str = "GET",
    headers: Mapping[str, str] | None = None,
    timeout: float | None = 20.0,
    request_cls: type[Request] = Request,
    urlopen_impl=urlopen,
) -> bytes:
    request_headers = dict(headers or {})
    request = request_cls(url, headers=request_headers, method=method.upper())
    try:
        with urlopen_impl(request, timeout=timeout) as response:
            return response.read()
    except (TimeoutError, socket.timeout) as exc:
        raise BotRuntimeError(f"{method.upper()} {url} timed out{_shared_timeout_phrase(timeout)}.") from exc
    except OverflowError as exc:
        raise BotRuntimeError(f"{method.upper()} {url} failed: invalid timeout value {timeout!r}.") from exc
    except HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace") if exc.fp is not None else ""
        raise BotRuntimeError(f"{method.upper()} {url} failed: {exc.code} {detail or exc.reason}") from exc
    except URLError as exc:
        raise BotRuntimeError(f"{method.upper()} {url} failed: {exc.reason}") from exc
    except (RemoteDisconnected, ConnectionResetError, BrokenPipeError, ConnectionAbortedError, OSError) as exc:
        raise BotRuntimeError(f"{method.upper()} {url} failed: {exc}") from exc


async def _async_json_request(
    url: str,
    *,
    method: str = "GET",
    payload: dict[str, Any] | None = None,
    headers: Mapping[str, str] | None = None,
    timeout: float | None = 20.0,
    client: httpx.AsyncClient | None = None,
) -> Any:
    request_headers = {"Accept": "application/json"}
    if payload is not None:
        request_headers["Content-Type"] = "application/json"
    if headers:
        request_headers.update({str(key): str(value) for key, value in headers.items()})
    http_client = client
    owns_client = http_client is None
    if http_client is None:
        http_client = httpx.AsyncClient()
    try:
        response = await http_client.request(
            method.upper(),
            url,
            json=payload,
            headers=request_headers,
            timeout=timeout,
        )
        response.raise_for_status()
    except httpx.TimeoutException as exc:
        raise BotRuntimeError(f"{method.upper()} {url} timed out{_shared_timeout_phrase(timeout)}.") from exc
    except OverflowError as exc:
        raise BotRuntimeError(f"{method.upper()} {url} failed: invalid timeout value {timeout!r}.") from exc
    except httpx.HTTPStatusError as exc:
        detail = exc.response.text if exc.response is not None else ""
        raise BotRuntimeError(
            f"{method.upper()} {url} failed: {exc.response.status_code} {detail or exc.response.reason_phrase}"
        ) from exc
    except httpx.RequestError as exc:
        raise BotRuntimeError(f"{method.upper()} {url} failed: {exc}") from exc
    finally:
        if owns_client:
            await http_client.aclose()
    raw = response.text
    if not raw.strip():
        return {}
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return raw


async def _async_multipart_json_request(
    url: str,
    *,
    fields: Mapping[str, object],
    file_field: str,
    file_name: str,
    file_bytes: bytes,
    mime_type: str,
    headers: Mapping[str, str] | None = None,
    timeout: float | None = 20.0,
    client: httpx.AsyncClient | None = None,
) -> Any:
    request_headers = {"Accept": "application/json"}
    if headers:
        request_headers.update({str(key): str(value) for key, value in headers.items()})
    form_fields = {str(key): str(value) for key, value in fields.items() if value is not None}
    files = {file_field: (file_name, file_bytes, mime_type)}
    http_client = client
    owns_client = http_client is None
    if http_client is None:
        http_client = httpx.AsyncClient()
    try:
        response = await http_client.request(
            "POST",
            url,
            data=form_fields,
            files=files,
            headers=request_headers,
            timeout=timeout,
        )
        response.raise_for_status()
    except httpx.TimeoutException as exc:
        raise BotRuntimeError(f"POST {url} timed out{_shared_timeout_phrase(timeout)}.") from exc
    except OverflowError as exc:
        raise BotRuntimeError(f"POST {url} failed: invalid timeout value {timeout!r}.") from exc
    except httpx.HTTPStatusError as exc:
        detail = exc.response.text if exc.response is not None else ""
        raise BotRuntimeError(
            f"POST {url} failed: {exc.response.status_code} {detail or exc.response.reason_phrase}"
        ) from exc
    except httpx.RequestError as exc:
        raise BotRuntimeError(f"POST {url} failed: {exc}") from exc
    finally:
        if owns_client:
            await http_client.aclose()
    raw = response.text
    if not raw.strip():
        return {}
    try:
        return json.loads(raw)
    except json.JSONDecodeError:
        return raw


async def _async_bytes_request(
    url: str,
    *,
    method: str = "GET",
    headers: Mapping[str, str] | None = None,
    timeout: float | None = 20.0,
    client: httpx.AsyncClient | None = None,
) -> bytes:
    request_headers = {str(key): str(value) for key, value in (headers or {}).items()}
    http_client = client
    owns_client = http_client is None
    if http_client is None:
        http_client = httpx.AsyncClient()
    try:
        response = await http_client.request(
            method.upper(),
            url,
            headers=request_headers,
            timeout=timeout,
        )
        response.raise_for_status()
    except httpx.TimeoutException as exc:
        raise BotRuntimeError(f"{method.upper()} {url} timed out{_shared_timeout_phrase(timeout)}.") from exc
    except OverflowError as exc:
        raise BotRuntimeError(f"{method.upper()} {url} failed: invalid timeout value {timeout!r}.") from exc
    except httpx.HTTPStatusError as exc:
        detail = exc.response.text if exc.response is not None else ""
        raise BotRuntimeError(
            f"{method.upper()} {url} failed: {exc.response.status_code} {detail or exc.response.reason_phrase}"
        ) from exc
    except httpx.RequestError as exc:
        raise BotRuntimeError(f"{method.upper()} {url} failed: {exc}") from exc
    finally:
        if owns_client:
            await http_client.aclose()
    return response.content


def parse_webhook_payload(raw_payload: str) -> list[dict[str, Any]]:
    if not raw_payload.strip():
        raise BotRuntimeError("No Telegram webhook payload provided on stdin.")
    try:
        payload = json.loads(raw_payload)
    except json.JSONDecodeError as exc:
        raise BotRuntimeError("Telegram webhook payload must be valid JSON.") from exc
    if isinstance(payload, list):
        updates = payload
    elif isinstance(payload, dict):
        updates = [payload]
    else:
        raise BotRuntimeError("Telegram webhook payload must be a JSON object or array.")
    if not updates:
        raise BotRuntimeError("Telegram webhook payload must contain at least one update.")
    normalized: list[dict[str, Any]] = []
    for index, update in enumerate(updates):
        if not isinstance(update, dict):
            raise BotRuntimeError(f"Telegram webhook update payload item #{index} must be a JSON object.")
        normalized.append(update)
    return normalized


__all__ = [
    "_async_bytes_request",
    "_async_json_request",
    "_async_multipart_json_request",
    "_bytes_request",
    "_clean_optional_str",
    "_compact_mapping",
    "_consume_pending_termination_context",
    "_json_request",
    "_multipart_json_request",
    "_parse_bool",
    "_parse_int",
    "_positive_int",
    "_runtime_backend_signature",
    "_runtime_link_fields",
    "_runtime_signature_from_link",
    "_signal_stop_suffix",
    "_termination_context_path",
    "parse_webhook_payload",
]
