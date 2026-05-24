from __future__ import annotations

import mimetypes
import tempfile
import threading
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence

from .config import BotConfig, BotRuntimeError
from .transport_io import _clean_optional_str
from .turn_inputs import _speech_turn_text


class LocalSpeechToTextError(BotRuntimeError):
    def __init__(self, user_message: str, *, detail: str | None = None):
        super().__init__(detail or user_message)
        self.user_message = user_message


@dataclass(frozen=True)
class LocalSpeechToTextTurnInput:
    text: str
    attachments: tuple[dict[str, Any], ...]


class LocalSpeechToTextRuntime:
    def __init__(self, config: BotConfig, *, telegram_api: Any):
        self.config = config
        self.telegram_api = telegram_api
        self._backend_module: Any | None = None
        self._backend_lock = threading.Lock()
        self._transcribe_lock = threading.Lock()

    def transcribe_message(
        self,
        message: Mapping[str, Any],
        *,
        attachments: Sequence[Mapping[str, Any]],
    ) -> LocalSpeechToTextTurnInput:
        if self.config.stt_mode != "local-stt":
            raise LocalSpeechToTextError(
                "Local STT is not enabled for this Telegram worker. Set `AIT_TELEGRAM_STT_MODE=local-stt` and retry."
            )
        if not attachments:
            raise LocalSpeechToTextError("No local-STT attachment was found in this Telegram message.")
        resolved_attachments = [dict(item) for item in attachments if isinstance(item, Mapping)]
        if not resolved_attachments:
            raise LocalSpeechToTextError("No local-STT attachment was found in this Telegram message.")
        file_id = _clean_optional_str(resolved_attachments[0].get("telegram_file_id"))
        if file_id is None:
            raise LocalSpeechToTextError("The Telegram voice attachment did not include a downloadable file id.")
        try:
            file_info = self.telegram_api.get_file(file_id)
            telegram_file_path = _clean_optional_str(file_info.get("file_path"))
            if telegram_file_path is not None:
                resolved_attachments[0]["telegram_file_path"] = telegram_file_path
            file_bytes = self.telegram_api.download_file_bytes(telegram_file_path or "")
        except AttributeError as exc:
            raise LocalSpeechToTextError(
                "Local STT is enabled, but this Telegram runtime cannot download the voice file on the current host.",
                detail=str(exc),
            ) from exc
        except BotRuntimeError as exc:
            raise LocalSpeechToTextError(
                "Local STT could not download that Telegram audio file. Please retry in a moment.",
                detail=str(exc),
            ) from exc

        temp_path = self._write_temp_audio_file(
            file_bytes=file_bytes,
            attachment=resolved_attachments[0],
        )
        try:
            transcript = self._transcribe_local_file(temp_path)
        finally:
            try:
                temp_path.unlink()
            except FileNotFoundError:
                pass
        caption = _clean_optional_str(message.get("caption"))
        return LocalSpeechToTextTurnInput(
            text=_speech_turn_text(caption, transcript),
            attachments=tuple(resolved_attachments),
        )

    def _load_backend(self) -> Any:
        with self._backend_lock:
            if self._backend_module is not None:
                return self._backend_module
            try:
                import mlx_whisper
            except ImportError as exc:
                raise LocalSpeechToTextError(
                    "Local STT requires `mlx-whisper` on the Telegram worker host. Install it there and retry.",
                    detail=str(exc),
                ) from exc
            try:
                self._backend_module = mlx_whisper
            except Exception as exc:  # pragma: no cover - depends on local runtime/device state
                raise LocalSpeechToTextError(
                    f"Local STT could not prepare `mlx-whisper` for model `{self.config.stt_model}`.",
                    detail=str(exc),
                ) from exc
            return self._backend_module

    def _transcribe_local_file(self, path: Path) -> str:
        backend = self._load_backend()
        transcribe_kwargs: dict[str, Any] = {
            "path_or_hf_repo": self.config.stt_model,
            "verbose": False,
        }
        if self.config.stt_language:
            transcribe_kwargs["language"] = self.config.stt_language
        try:
            with self._transcribe_lock:
                result = backend.transcribe(str(path), **transcribe_kwargs)
                transcript = str((result or {}).get("text", "")).strip()
        except LocalSpeechToTextError:
            raise
        except Exception as exc:  # pragma: no cover - depends on local runtime/device state
            raise LocalSpeechToTextError(
                "Local STT failed while transcribing that audio. Please retry or send text instead.",
                detail=str(exc),
            ) from exc
        if not transcript:
            raise LocalSpeechToTextError(
                "Local STT could not hear any speech in that message. Please retry or send text instead."
            )
        return transcript

    def _write_temp_audio_file(
        self,
        *,
        file_bytes: bytes,
        attachment: Mapping[str, Any],
    ) -> Path:
        suffix = (
            Path(str(attachment.get("telegram_file_path") or "")).suffix
            or Path(str(attachment.get("file_name") or "")).suffix
            or mimetypes.guess_extension(str(attachment.get("mime_type") or "")) or ".audio"
        )
        temp_dir = self.config.sync_state_path.parent
        temp_dir.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(
            prefix="ait-telegram-stt-",
            suffix=suffix,
            dir=temp_dir,
            delete=False,
        ) as handle:
            handle.write(file_bytes)
            return Path(handle.name)

