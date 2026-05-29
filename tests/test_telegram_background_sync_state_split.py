from __future__ import annotations

from pathlib import Path

from ait_agent.telegram.background_sync import TelegramBackgroundSyncManager

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_telegram_background_sync_manager_owns_state_helpers() -> None:
    assert hasattr(TelegramBackgroundSyncManager, "background_sync_backoff_active")
    assert hasattr(TelegramBackgroundSyncManager, "record_background_sync_success")
    assert hasattr(TelegramBackgroundSyncManager, "record_background_sync_failure")


def test_telegram_app_no_longer_owns_background_sync_state_helpers() -> None:
    text = (WORKSPACE_ROOT / "src/ait_agent/telegram/app.py").read_text(encoding="utf-8")
    assert "from .background_sync import TelegramBackgroundSyncManager" in text
    assert "def _background_sync_backoff_active" not in text
    assert "def _record_background_sync_success" not in text
    assert "def _record_background_sync_failure" not in text
