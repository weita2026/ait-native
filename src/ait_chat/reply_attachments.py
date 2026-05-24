from __future__ import annotations

import json
import mimetypes
import re
from pathlib import Path
from typing import Any, Mapping, Sequence

DISCORD_ATTACHMENT_BLOCK_RE = re.compile(r"```ait-attachments\s*(.*?)```", re.IGNORECASE | re.DOTALL)


def _normalize_discord_reply_attachment(
    value: object,
    *,
    repo_root: Path,
) -> dict[str, Any] | None:
    if not isinstance(value, Mapping):
        return None
    raw_local_path = str(value.get("local_path") or value.get("path") or "").strip()
    if not raw_local_path:
        return None
    candidate = Path(raw_local_path)
    if not candidate.is_absolute():
        candidate = repo_root / candidate
    try:
        resolved_repo_root = repo_root.resolve()
        resolved_candidate = candidate.resolve()
    except OSError:
        return None
    try:
        resolved_candidate.relative_to(resolved_repo_root)
    except ValueError:
        return None
    if not resolved_candidate.exists() or not resolved_candidate.is_file():
        return None
    file_name = str(value.get("file_name") or "").strip() or resolved_candidate.name
    file_name = Path(file_name).name or resolved_candidate.name
    mime_type = str(value.get("mime_type") or "").strip() or mimetypes.guess_type(file_name)[0] or "application/octet-stream"
    caption = str(value.get("caption") or "").strip() or None
    attachment = {
        "kind": str(value.get("kind") or "document").strip().lower() or "document",
        "local_path": str(resolved_candidate),
        "file_name": file_name,
        "mime_type": mime_type,
    }
    if caption:
        attachment["caption"] = caption
    return attachment


def extract_discord_reply_attachments(text: str, *, repo_root: Path | None) -> tuple[str, tuple[dict[str, Any], ...]]:
    if repo_root is None:
        return str(text or "").strip(), ()
    raw_text = str(text or "")
    matches = list(DISCORD_ATTACHMENT_BLOCK_RE.finditer(raw_text))
    if not matches:
        return raw_text.strip(), ()
    attachments: list[dict[str, Any]] = []
    for match in matches:
        body = str(match.group(1) or "").strip()
        if not body:
            continue
        try:
            payload = json.loads(body)
        except json.JSONDecodeError:
            continue
        if not isinstance(payload, Sequence) or isinstance(payload, (str, bytes, bytearray)):
            continue
        for item in payload[:8]:
            normalized = _normalize_discord_reply_attachment(item, repo_root=repo_root)
            if normalized is not None:
                attachments.append(normalized)
    cleaned = DISCORD_ATTACHMENT_BLOCK_RE.sub("", raw_text).strip()
    return cleaned, tuple(attachments)


__all__ = ["extract_discord_reply_attachments"]
