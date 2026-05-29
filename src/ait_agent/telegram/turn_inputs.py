from __future__ import annotations

from pathlib import Path
import re
from typing import Any, Mapping, Sequence


MUSIC_DOCUMENT_EXTENSIONS = {
    ".aac",
    ".aif",
    ".aiff",
    ".alac",
    ".flac",
    ".m4a",
    ".mp3",
    ".ogg",
    ".opus",
    ".wav",
    ".wma",
}

PHOTO_DOCUMENT_EXTENSIONS = {
    ".jpg",
    ".jpeg",
    ".png",
    ".webp",
}


def _clean_optional_str(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _positive_int(value: object) -> int | None:
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    return parsed if parsed > 0 else None


def strip_leading_bot_mention(text: str, username: str) -> str:
    if not username:
        return text
    return re.sub(rf"^@{re.escape(username)}(?:\s+|[:,，：-]+)", "", text.strip(), count=1, flags=re.IGNORECASE)


def normalize_user_text(text: str, username: str) -> str:
    normalized = strip_leading_bot_mention(str(text or ""), username)
    normalized = re.sub(r"\r\n?", "\n", normalized)
    normalized = re.sub(r"[ \t]+", " ", normalized)
    normalized = re.sub(r"\n{3,}", "\n\n", normalized)
    return normalized.strip()


def _format_file_size(size_bytes: object) -> str | None:
    size = _positive_int(size_bytes)
    if size is None:
        return None
    if size < 1024:
        return f"{size} B"
    if size < 1024 * 1024:
        return f"{size / 1024:.1f} KB"
    return f"{size / (1024 * 1024):.1f} MB"


def _is_supported_music_document(document: Mapping[str, Any]) -> bool:
    mime_type = str(document.get("mime_type") or "").strip().lower()
    if mime_type.startswith("audio/"):
        return True
    suffix = Path(str(document.get("file_name") or "").strip().lower()).suffix
    return suffix in MUSIC_DOCUMENT_EXTENSIONS


def _speech_attachments_from_message(
    message: Mapping[str, Any],
    *,
    include_audio_uploads: bool,
) -> list[dict[str, Any]]:
    attachments: list[dict[str, Any]] = []
    caption = _clean_optional_str(message.get("caption"))
    voice = message.get("voice")
    if isinstance(voice, Mapping):
        attachments.append(
            {
                "kind": "voice",
                "media_kind": "speech",
                "telegram_file_id": _clean_optional_str(voice.get("file_id")),
                "telegram_file_unique_id": _clean_optional_str(voice.get("file_unique_id")),
                "mime_type": _clean_optional_str(voice.get("mime_type")),
                "caption": caption,
                "duration_seconds": _positive_int(voice.get("duration")),
                "file_size_bytes": _positive_int(voice.get("file_size")),
            }
        )
    if include_audio_uploads:
        audio = message.get("audio")
        if isinstance(audio, Mapping):
            attachments.append(
                {
                    "kind": "audio",
                    "media_kind": "speech",
                    "telegram_file_id": _clean_optional_str(audio.get("file_id")),
                    "telegram_file_unique_id": _clean_optional_str(audio.get("file_unique_id")),
                    "file_name": _clean_optional_str(audio.get("file_name")),
                    "mime_type": _clean_optional_str(audio.get("mime_type")),
                    "caption": caption,
                    "title": _clean_optional_str(audio.get("title")),
                    "performer": _clean_optional_str(audio.get("performer")),
                    "duration_seconds": _positive_int(audio.get("duration")),
                    "file_size_bytes": _positive_int(audio.get("file_size")),
                }
            )
        document = message.get("document")
        if isinstance(document, Mapping) and _is_supported_music_document(document):
            attachments.append(
                {
                    "kind": "document",
                    "media_kind": "speech",
                    "telegram_file_id": _clean_optional_str(document.get("file_id")),
                    "telegram_file_unique_id": _clean_optional_str(document.get("file_unique_id")),
                    "file_name": _clean_optional_str(document.get("file_name")),
                    "mime_type": _clean_optional_str(document.get("mime_type")),
                    "caption": caption,
                    "file_size_bytes": _positive_int(document.get("file_size")),
                }
            )
    return [attachment for attachment in attachments if attachment.get("telegram_file_id")]


def _music_attachments_from_message(message: Mapping[str, Any]) -> list[dict[str, Any]]:
    attachments: list[dict[str, Any]] = []
    caption = _clean_optional_str(message.get("caption"))
    audio = message.get("audio")
    if isinstance(audio, Mapping):
        attachments.append(
            {
                "kind": "audio",
                "media_kind": "music",
                "telegram_file_id": _clean_optional_str(audio.get("file_id")),
                "telegram_file_unique_id": _clean_optional_str(audio.get("file_unique_id")),
                "file_name": _clean_optional_str(audio.get("file_name")),
                "mime_type": _clean_optional_str(audio.get("mime_type")),
                "caption": caption,
                "title": _clean_optional_str(audio.get("title")),
                "performer": _clean_optional_str(audio.get("performer")),
                "duration_seconds": _positive_int(audio.get("duration")),
                "file_size_bytes": _positive_int(audio.get("file_size")),
            }
        )
    document = message.get("document")
    if isinstance(document, Mapping) and _is_supported_music_document(document):
        attachments.append(
            {
                "kind": "document",
                "media_kind": "music",
                "telegram_file_id": _clean_optional_str(document.get("file_id")),
                "telegram_file_unique_id": _clean_optional_str(document.get("file_unique_id")),
                "file_name": _clean_optional_str(document.get("file_name")),
                "mime_type": _clean_optional_str(document.get("mime_type")),
                "caption": caption,
                "file_size_bytes": _positive_int(document.get("file_size")),
            }
        )
    return [attachment for attachment in attachments if attachment.get("telegram_file_id")]


def _attachment_summary(attachments: Sequence[Mapping[str, Any]]) -> str:
    if not attachments:
        return ""
    first_media_kind = str(attachments[0].get("media_kind") or "").strip().lower()
    if first_media_kind != "music":
        return ""
    lines = ["Telegram music upload:"]
    for attachment in attachments:
        label = (
            attachment.get("file_name")
            or attachment.get("title")
            or attachment.get("performer")
            or attachment.get("telegram_file_id")
            or "uploaded-audio"
        )
        details: list[str] = []
        if attachment.get("title") and attachment.get("title") != label:
            details.append(f"title={attachment['title']}")
        if attachment.get("performer"):
            details.append(f"performer={attachment['performer']}")
        if attachment.get("mime_type"):
            details.append(str(attachment["mime_type"]))
        if attachment.get("duration_seconds"):
            details.append(f"{attachment['duration_seconds']}s")
        size_label = _format_file_size(attachment.get("file_size_bytes"))
        if size_label:
            details.append(size_label)
        detail_suffix = f" ({', '.join(details)})" if details else ""
        lines.append(f"- {label}{detail_suffix}")
    return "\n".join(lines)


def _normalized_turn_text(
    *,
    raw_text: str | None,
    username: str,
    attachments: list[dict[str, Any]] | None = None,
) -> str:
    normalized = normalize_user_text(raw_text or "", username) if raw_text else ""
    attachment_summary = _attachment_summary(attachments or [])
    if attachment_summary:
        return f"{normalized}\n\n{attachment_summary}".strip() if normalized else attachment_summary
    return normalized


def _normalize_speech_transcript(text: str) -> str:
    normalized = re.sub(r"\r\n?", "\n", str(text or ""))
    normalized = re.sub(r"[ \t]+", " ", normalized)
    normalized = re.sub(r"\n{3,}", "\n\n", normalized)
    return normalized.strip()


def _speech_turn_text(caption: str | None, transcript: str) -> str:
    normalized_transcript = _normalize_speech_transcript(transcript)
    if caption and normalized_transcript:
        return f"{caption.strip()}\n\n[local speech transcript]\n{normalized_transcript}".strip()
    return (caption or normalized_transcript).strip()


def _transport_reply_attachments(assistant_event: Mapping[str, Any] | None) -> list[dict[str, Any]]:
    if not isinstance(assistant_event, Mapping):
        return []
    payload = assistant_event.get("payload")
    if not isinstance(payload, Mapping):
        return []
    envelope = payload.get("transport_reply_envelope")
    if not isinstance(envelope, Mapping):
        return []
    message = envelope.get("message")
    if not isinstance(message, Mapping):
        return []
    attachments = message.get("attachments")
    if not isinstance(attachments, list):
        return []
    return [dict(item) for item in attachments if isinstance(item, Mapping)]


def _transport_reply_text(assistant_event: Mapping[str, Any] | None) -> str:
    if not isinstance(assistant_event, Mapping):
        return ""
    payload = assistant_event.get("payload")
    if not isinstance(payload, Mapping):
        return ""
    envelope = payload.get("transport_reply_envelope")
    if isinstance(envelope, Mapping):
        message = envelope.get("message")
        if isinstance(message, Mapping):
            envelope_text = str(message.get("text") or "").strip()
            if envelope_text:
                return envelope_text
    return str(payload.get("text") or "").strip()


def _attachment_should_send_as_audio(attachment: Mapping[str, Any]) -> bool:
    kind = str(attachment.get("kind") or "").strip().lower()
    if kind == "audio":
        return True
    if kind == "document":
        return False
    mime_type = str(attachment.get("mime_type") or "").strip().lower()
    if mime_type.startswith("audio/"):
        return True
    suffix = Path(str(attachment.get("file_name") or attachment.get("local_path") or "").strip().lower()).suffix
    return suffix in MUSIC_DOCUMENT_EXTENSIONS and kind != "document"


def _attachment_should_send_as_photo(attachment: Mapping[str, Any]) -> bool:
    kind = str(attachment.get("kind") or "").strip().lower()
    if kind == "document":
        return False
    if kind in {"photo", "image"}:
        return True
    mime_type = str(attachment.get("mime_type") or "").strip().lower()
    if mime_type.startswith("image/") and mime_type != "image/gif":
        return True
    suffix = Path(str(attachment.get("file_name") or attachment.get("local_path") or "").strip().lower()).suffix
    return suffix in PHOTO_DOCUMENT_EXTENSIONS
