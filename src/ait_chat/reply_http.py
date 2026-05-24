from __future__ import annotations

import json
import socket
from collections.abc import Callable
from typing import Any
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen


def json_request(
    url: str,
    *,
    method: str = "GET",
    payload: dict[str, Any] | None = None,
    headers: dict[str, str] | None = None,
    timeout: float | None = 20.0,
    urlopen_fn: Callable[..., Any] = urlopen,
    error_cls: type[Exception],
    timeout_phrase: Callable[[float | None], str],
) -> Any:
    body = None
    request_headers = {"Accept": "application/json"}
    if payload is not None:
        body = json.dumps(payload).encode("utf-8")
        request_headers["Content-Type"] = "application/json"
    if headers:
        request_headers.update(headers)
    request = Request(url, data=body, headers=request_headers, method=method)
    try:
        with urlopen_fn(request, timeout=timeout) as response:
            raw = response.read()
            if not raw:
                return None
            return json.loads(raw.decode("utf-8"))
    except (TimeoutError, socket.timeout) as exc:
        raise error_cls(f"{method} {url} timed out{timeout_phrase(timeout)}.") from exc
    except OverflowError as exc:
        raise error_cls(f"{method} {url} failed: invalid timeout value {timeout!r}.") from exc
    except HTTPError as exc:
        detail = exc.read().decode("utf-8", errors="replace")
        raise error_cls(f"{method} {url} failed: {exc.code} {detail}") from exc
    except URLError as exc:
        raise error_cls(f"{method} {url} failed: {exc.reason}") from exc


def response_output_text(payload: dict[str, Any]) -> str:
    blocks = payload.get("output") or []
    texts: list[str] = []
    for item in blocks:
        if not isinstance(item, dict) or item.get("type") != "message":
            continue
        for part in item.get("content") or []:
            if not isinstance(part, dict) or part.get("type") != "output_text":
                continue
            text = str(part.get("text") or "").strip()
            if text:
                texts.append(text)
    return "\n\n".join(texts).strip()


__all__ = ["json_request", "response_output_text"]
