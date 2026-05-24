from __future__ import annotations

from typing import Any


class TelegramUpdateDispatch:
    def dispatch_key(self, update: dict[str, Any]) -> str:
        chat_id = self.chat_id_from_update(update)
        if chat_id is not None:
            return self.dispatch_key_for_chat(chat_id)
        update_id = int(update.get("update_id") or 0)
        if update_id:
            return f"update-{update_id}"
        return "update-unknown"

    def dispatch_key_for_chat(self, chat_id: str | int) -> str:
        return f"chat-{chat_id}"

    def chat_id_from_update(self, update: dict[str, Any]) -> str | int | None:
        message = update.get("message") or {}
        chat = message.get("chat") or {}
        return chat.get("id")

    def update_key(self, update: dict[str, Any]) -> str:
        update_id = int(update.get("update_id") or 0)
        if update_id:
            return f"update-{update_id}"
        message = update.get("message") or {}
        message_id = int(message.get("message_id") or 0)
        if message_id:
            return f"message-{message_id}"
        return f"memory-{id(update)}"
