from __future__ import annotations

from pathlib import Path

from ait_agent.telegram import app as telegram_app
from ait_agent.telegram import live_replies as telegram_live_replies

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_telegram_live_reply_manager_stays_reexported() -> None:
    assert telegram_app.TelegramLiveReplyManager is telegram_live_replies.TelegramLiveReplyManager


def test_telegram_app_imports_live_reply_module() -> None:
    text = (WORKSPACE_ROOT / "src/ait_agent/telegram/app.py").read_text(encoding="utf-8")
    assert "from .live_replies import TelegramLiveReplyManager" in text
    assert "self._live_reply_manager = TelegramLiveReplyManager(" in text
