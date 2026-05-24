from __future__ import annotations

import os
import signal
import sys
from pathlib import Path
from typing import TYPE_CHECKING

from .config import BotConfig, BotRuntimeError, load_config
from .transport_io import _signal_stop_suffix, parse_webhook_payload

if TYPE_CHECKING:
    from .app import TelegramBotService


def _telegram_bot_service_cls() -> type["TelegramBotService"]:
    from .app import TelegramBotService

    return TelegramBotService


def run_webhook_updates(
    raw_payload: str,
    *,
    service: TelegramBotService | None = None,
    config: BotConfig | None = None,
    repo_root: Path | None = None,
) -> int:
    updates = parse_webhook_payload(raw_payload)
    if service is not None:
        bot_service = service
    else:
        resolved_root = repo_root or Path(os.environ.get("AIT_REPO_ROOT") or Path.cwd())
        bot_service = _telegram_bot_service_cls()(config or load_config(resolved_root))
    for update in updates:
        bot_service.handle_update(update)
    return len(updates)


def _install_signal_handlers(service: TelegramBotService) -> None:
    def _handler(signum, _frame):
        print(f"Received signal {signum}; stopping ait Telegram bot{_signal_stop_suffix(signum)}.", flush=True)
        service.stop()

    signal.signal(signal.SIGTERM, _handler)
    signal.signal(signal.SIGINT, _handler)


def webhook_main() -> None:
    repo_root = Path(os.environ.get("AIT_REPO_ROOT") or Path.cwd())
    raw_payload = sys.stdin.read()
    try:
        run_webhook_updates(raw_payload, repo_root=repo_root)
    except BotRuntimeError as exc:
        print(f"ait Telegram webhook failed: {exc}", file=sys.stderr, flush=True)
        raise SystemExit(1) from exc
    except Exception as exc:  # pragma: no cover - defensive crash logging for webhook handler
        print(f"ait Telegram webhook crashed: {exc}", file=sys.stderr, flush=True)
        raise


def main() -> None:
    repo_root = Path(os.environ.get("AIT_REPO_ROOT") or Path.cwd())
    config = load_config(repo_root)
    service = _telegram_bot_service_cls()(config)
    _install_signal_handlers(service)
    print(
        f"ait Telegram bot starting · repo={config.repo_name} · backend={config.runtime_mode}"
        f"{f' · remote={config.runtime_remote_name} · server={config.ait_server_url}' if config.ait_server_url else ''}"
        f" · state={config.sync_state_path}",
        flush=True,
    )
    try:
        service.run_forever()
    except BotRuntimeError as exc:
        print(f"ait Telegram bot failed: {exc}", file=sys.stderr, flush=True)
        raise SystemExit(1) from exc
    except Exception as exc:  # pragma: no cover - defensive crash logging for daemon mode
        print(f"ait Telegram bot crashed: {exc}", file=sys.stderr, flush=True)
        raise
