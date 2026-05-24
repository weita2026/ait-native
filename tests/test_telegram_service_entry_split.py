from __future__ import annotations

from pathlib import Path

import ait_agent.telegram.app as telegram_app
import ait_agent.telegram.service_entry as telegram_service_entry


def test_telegram_app_reexports_service_entry_helpers() -> None:
    assert telegram_app.run_webhook_updates is telegram_service_entry.run_webhook_updates
    assert telegram_app.webhook_main is telegram_service_entry.webhook_main
    assert telegram_app.main is telegram_service_entry.main


def test_telegram_app_imports_service_entry_module() -> None:
    text = Path("src/ait_agent/telegram/app.py").read_text(encoding="utf-8")
    assert "from .service_entry import (" in text
    assert "run_webhook_updates" in text
    assert "webhook_main" in text
    assert "main" in text
