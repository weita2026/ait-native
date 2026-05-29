from __future__ import annotations

from pathlib import Path

from ait_agent.telegram import app as telegram_app
from ait_agent.telegram import reply_turns as telegram_reply_turns

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_telegram_reply_spool_helpers_stay_reexported() -> None:
    assert telegram_app.PendingTelegramReplyTurn is telegram_reply_turns.PendingTelegramReplyTurn
    assert telegram_app.TelegramReplyTurnSpool is telegram_reply_turns.TelegramReplyTurnSpool


def test_telegram_app_imports_reply_turn_spool_module() -> None:
    text = (WORKSPACE_ROOT / "src/ait_agent/telegram/app.py").read_text(encoding="utf-8")
    assert "from .reply_turns import PendingTelegramReplyTurn, TelegramReplyTurnManager, TelegramReplyTurnSpool" in text
    assert "self._reply_turn_spool = TelegramReplyTurnSpool(" in text
    assert "return self._reply_turn_spool.pending_turn_spool_key(pending_turn)" in text
    assert "return self._reply_turn_spool.telegram_reply_spool_entries(link)" in text
    assert "return self._reply_turn_spool.remember_pending_reply_turn(" in text
    assert "return self._reply_turn_spool.clear_pending_reply_turn_spool_entry(pending_turn)" in text
