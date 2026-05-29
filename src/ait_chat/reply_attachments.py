from __future__ import annotations

import json
import mimetypes
import re
from pathlib import Path
from typing import Any, Mapping, Sequence


ATTACHMENT_BLOCK_RE = re.compile(r"```ait-attachments\s*(.*?)```", re.IGNORECASE | re.DOTALL)
TELEGRAM_PHOTO_EXTENSIONS = frozenset({".jpg", ".jpeg", ".png", ".webp"})
TELEGRAM_IMAGE_DOCUMENT_EXTENSIONS = TELEGRAM_PHOTO_EXTENSIONS | frozenset({".gif"})


def _normalize_repo_contained_path(raw_local_path: str, *, repo_root: Path) -> Path | None:
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
    return resolved_candidate


def _telegram_attachment_kind(*, kind: str, mime_type: str, file_name: str) -> str | None:
    normalized_kind = kind or ""
    suffix = Path(file_name.lower()).suffix
    is_image_candidate = mime_type.startswith("image/") or suffix in TELEGRAM_IMAGE_DOCUMENT_EXTENSIONS
    if not is_image_candidate:
        return None
    if normalized_kind == "document":
        return "document"
    if mime_type.startswith("image/") and mime_type != "image/gif":
        return "photo"
    if suffix in TELEGRAM_PHOTO_EXTENSIONS:
        return "photo"
    return "document"


def _normalize_reply_attachment(
    value: object,
    *,
    repo_root: Path,
    surface: str,
) -> dict[str, Any] | None:
    if not isinstance(value, Mapping):
        return None
    raw_local_path = str(value.get("local_path") or value.get("path") or "").strip()
    if not raw_local_path:
        return None
    resolved_candidate = _normalize_repo_contained_path(raw_local_path, repo_root=repo_root)
    if resolved_candidate is None:
        return None
    file_name = str(value.get("file_name") or "").strip() or resolved_candidate.name
    file_name = Path(file_name).name or resolved_candidate.name
    mime_type = str(value.get("mime_type") or "").strip() or mimetypes.guess_type(file_name)[0] or "application/octet-stream"
    caption = str(value.get("caption") or "").strip() or None
    normalized_surface = str(surface or "").strip().lower()
    requested_kind = str(value.get("kind") or "").strip().lower()
    if normalized_surface == "telegram":
        kind = _telegram_attachment_kind(kind=requested_kind, mime_type=mime_type.lower(), file_name=file_name)
        if kind is None:
            return None
    elif normalized_surface == "discord":
        kind = requested_kind or "document"
    else:
        return None
    attachment = {
        "kind": kind,
        "local_path": str(resolved_candidate),
        "file_name": file_name,
        "mime_type": mime_type,
    }
    if caption:
        attachment["caption"] = caption
    return attachment


def extract_reply_attachments(
    text: str,
    *,
    repo_root: Path | None,
    surface: str,
) -> tuple[str, tuple[dict[str, Any], ...]]:
    if repo_root is None:
        return str(text or "").strip(), ()
    normalized_surface = str(surface or "").strip().lower()
    if normalized_surface not in {"discord", "telegram"}:
        return str(text or "").strip(), ()
    raw_text = str(text or "")
    matches = list(ATTACHMENT_BLOCK_RE.finditer(raw_text))
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
            normalized = _normalize_reply_attachment(item, repo_root=repo_root, surface=normalized_surface)
            if normalized is not None:
                attachments.append(normalized)
    cleaned = ATTACHMENT_BLOCK_RE.sub("", raw_text).strip()
    return cleaned, tuple(attachments)


def extract_discord_reply_attachments(text: str, *, repo_root: Path | None) -> tuple[str, tuple[dict[str, Any], ...]]:
    return extract_reply_attachments(text, repo_root=repo_root, surface="discord")


def extract_telegram_reply_attachments(text: str, *, repo_root: Path | None) -> tuple[str, tuple[dict[str, Any], ...]]:
    return extract_reply_attachments(text, repo_root=repo_root, surface="telegram")


__all__ = [
    "extract_discord_reply_attachments",
    "extract_reply_attachments",
    "extract_telegram_reply_attachments",
]
