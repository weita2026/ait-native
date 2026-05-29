from __future__ import annotations

from pathlib import Path

from ait_agent.telegram import app as telegram_app
from ait_agent.telegram import logical_turns as telegram_logical_turns

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_telegram_logical_turn_helpers_stay_reexported() -> None:
    assert telegram_app.PendingTelegramTextUpdate is telegram_logical_turns.PendingTelegramTextUpdate
    assert telegram_app.TelegramLogicalTurn is telegram_logical_turns.TelegramLogicalTurn


def test_telegram_app_imports_logical_turn_module() -> None:
    text = (WORKSPACE_ROOT / "src/ait_agent/telegram/app.py").read_text(encoding="utf-8")
    assert "from .logical_turns import (" in text
    assert "PendingTelegramTextUpdate" in text
    assert "TelegramLogicalTurn" in text
    assert "TelegramLogicalTurnBuffer" in text
