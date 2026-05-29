from __future__ import annotations

from pathlib import Path

from ait_agent.telegram import app as telegram_app
from ait_agent.telegram import service_runtime as telegram_service_runtime

WORKSPACE_ROOT = Path(__file__).resolve().parents[1]


def test_telegram_service_runtime_stays_reexported() -> None:
    assert telegram_app.TelegramServiceRuntime is telegram_service_runtime.TelegramServiceRuntime


def test_telegram_app_imports_service_runtime_module() -> None:
    text = (WORKSPACE_ROOT / "src/ait_agent/telegram/app.py").read_text(encoding="utf-8")
    assert "from .service_runtime import TelegramServiceRuntime" in text
    assert "self._service_runtime = TelegramServiceRuntime(" in text
    assert "return self._service_runtime.submit_serialized(queue_key, fn, *args)" in text
    assert "return self._service_runtime.run_due_background_sync(next_background_sync_at)" in text
