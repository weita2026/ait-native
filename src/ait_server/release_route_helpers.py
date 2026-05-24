from __future__ import annotations

from typing import Any

from fastapi import Request, Response


def _absolute_request_url(request: Request, path: str | None) -> str | None:
    if not path:
        return None
    return str(request.base_url).rstrip("/") + str(path)


def _release_response_payload(request: Request, payload: dict[str, Any]) -> dict[str, Any]:
    out = dict(payload)
    artifacts: list[dict[str, Any]] = []
    for artifact in payload.get("artifacts", []):
        if not isinstance(artifact, dict):
            continue
        row = dict(artifact)
        row["download_url"] = _absolute_request_url(request, row.get("download_path"))
        artifacts.append(row)
    out["artifacts"] = artifacts
    formula = out.get("formula") if isinstance(out.get("formula"), dict) else {}
    if formula:
        if str(formula.get("url") or "").startswith("/"):
            formula["url"] = _absolute_request_url(request, str(formula["url"]))
        if str(formula.get("download_path") or "").startswith("/"):
            formula["download_url"] = _absolute_request_url(request, str(formula["download_path"]))
    out["formula"] = formula
    return out


def _release_artifact_response(
    payload: dict[str, Any],
    *,
    release_id: str,
    artifact_kind: str,
) -> Response:
    artifact = payload["artifact"]
    filename = str(artifact.get("download_name") or artifact.get("path") or f"{release_id}-{artifact_kind}")
    headers = {
        "Content-Disposition": f'attachment; filename="{filename}"',
        "ETag": str(artifact.get("sha256") or ""),
    }
    return Response(
        content=payload["data"],
        media_type=str(artifact.get("media_type") or "application/octet-stream"),
        headers=headers,
    )


__all__ = [
    "_absolute_request_url",
    "_release_artifact_response",
    "_release_response_payload",
]
