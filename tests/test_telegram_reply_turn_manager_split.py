from __future__ import annotations

from pathlib import Path

from ait_agent.telegram import app as telegram_app
from ait_agent.telegram import reply_turns as telegram_reply_turns


def test_telegram_reply_turn_helpers_stay_reexported() -> None:
    assert telegram_app.PendingTelegramReplyTurn is telegram_reply_turns.PendingTelegramReplyTurn


def test_telegram_app_imports_reply_turn_module() -> None:
    text = Path("src/ait_agent/telegram/app.py").read_text(encoding="utf-8")
    assert "from .reply_turns import PendingTelegramReplyTurn, TelegramReplyTurnManager, TelegramReplyTurnSpool" in text
