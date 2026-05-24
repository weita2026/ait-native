from __future__ import annotations

from pathlib import Path

from ait_agent.telegram import app as telegram_app
from ait_agent.telegram import update_dispatch as telegram_update_dispatch


def test_telegram_update_dispatch_helpers_stay_reexported() -> None:
    assert telegram_app.TelegramUpdateDispatch is telegram_update_dispatch.TelegramUpdateDispatch


def test_telegram_app_imports_update_dispatch_module() -> None:
    text = Path("src/ait_agent/telegram/app.py").read_text(encoding="utf-8")
    assert "from .update_dispatch import TelegramUpdateDispatch" in text
    assert "self._update_dispatch = TelegramUpdateDispatch()" in text
    assert "return self._update_dispatch.dispatch_key(update)" in text
    assert "return self._update_dispatch.dispatch_key_for_chat(chat_id)" in text
    assert "return self._update_dispatch.chat_id_from_update(update)" in text
    assert "return self._update_dispatch.update_key(update)" in text
