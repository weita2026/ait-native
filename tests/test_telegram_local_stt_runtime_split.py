from __future__ import annotations

from ait_agent.telegram import app as telegram_app
from ait_agent.telegram import speech_to_text as telegram_speech_to_text


def test_telegram_local_stt_runtime_helpers_stay_reexported() -> None:
    assert telegram_app.LocalSpeechToTextError is telegram_speech_to_text.LocalSpeechToTextError
    assert telegram_app.LocalSpeechToTextTurnInput is telegram_speech_to_text.LocalSpeechToTextTurnInput
    assert telegram_app.LocalSpeechToTextRuntime is telegram_speech_to_text.LocalSpeechToTextRuntime

