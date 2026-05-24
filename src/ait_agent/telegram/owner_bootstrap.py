from __future__ import annotations

from typing import Any, Callable, Mapping


def _compact_mapping(payload: Mapping[str, Any]) -> dict[str, Any]:
    compact: dict[str, Any] = {}
    for key, value in payload.items():
        if value is None:
            continue
        if isinstance(value, Mapping):
            nested = _compact_mapping(value)
            if nested:
                compact[key] = nested
            continue
        if isinstance(value, list) and not value:
            continue
        compact[key] = value
    return compact


def _telegram_bootstrap_owner_user_id(state: Mapping[str, Any] | None) -> str | None:
    owner_user_id = str((state or {}).get("owner_user_id") or "").strip()
    return owner_user_id or None


def _telegram_bootstrap_pending_user_id(state: Mapping[str, Any] | None) -> str | None:
    pending_user_id = str((state or {}).get("pending_user_id") or "").strip()
    return pending_user_id or None


def _telegram_bootstrap_failed_attempts(state: Mapping[str, Any] | None) -> dict[str, int]:
    raw = (state or {}).get("failed_attempts")
    if not isinstance(raw, Mapping):
        return {}
    attempts: dict[str, int] = {}
    for user_id, value in raw.items():
        try:
            count = int(value)
        except (TypeError, ValueError):
            continue
        if count > 0:
            attempts[str(user_id)] = count
    return attempts


def _telegram_bootstrap_blacklist(state: Mapping[str, Any] | None) -> dict[str, dict[str, Any]]:
    raw = (state or {}).get("blacklist")
    if not isinstance(raw, Mapping):
        return {}
    blacklist: dict[str, dict[str, Any]] = {}
    for user_id, payload in raw.items():
        blacklist[str(user_id)] = dict(payload) if isinstance(payload, Mapping) else {}
    return blacklist


class TelegramOwnerBootstrapGate:
    def __init__(
        self,
        *,
        config: Any,
        telegram_api: Any,
        state_get_chat: Callable[[str | int], dict[str, Any] | None],
        state_get_bootstrap_auth: Callable[[], dict[str, Any]],
        state_save_bootstrap_auth: Callable[[dict[str, Any]], dict[str, Any]],
        now_iso: Callable[[], str],
        user_display_name: Callable[[Mapping[str, Any]], str | None],
    ) -> None:
        self._config = config
        self._telegram_api = telegram_api
        self._state_get_chat = state_get_chat
        self._state_get_bootstrap_auth = state_get_bootstrap_auth
        self._state_save_bootstrap_auth = state_save_bootstrap_auth
        self._now_iso = now_iso
        self._user_display_name = user_display_name

    def handle(
        self,
        chat_id: str | int,
        chat: Mapping[str, Any],
        from_user: Mapping[str, Any],
        chat_title: str,
        *,
        raw_text: str | None,
        command: tuple[str, str] | None,
        attachments_present: bool,
    ) -> bool:
        if not self._config.owner_bootstrap_enabled:
            return False
        user_id = str(from_user.get("id") or "").strip()
        if not user_id:
            return True
        auth_state = self._state_get_bootstrap_auth()
        owner_user_id = _telegram_bootstrap_owner_user_id(auth_state)
        if owner_user_id:
            return user_id != owner_user_id
        blacklist = _telegram_bootstrap_blacklist(auth_state)
        if user_id in blacklist:
            return True
        pending_user_id = _telegram_bootstrap_pending_user_id(auth_state)
        if pending_user_id and pending_user_id != user_id:
            return True
        if pending_user_id is None and self._adopt_owner_from_existing_private_link(
            chat_id,
            chat,
            from_user,
            chat_title,
            auth_state,
        ):
            return False
        command_name = command[0] if command else None
        message_text = str(raw_text or "").strip()
        if command_name == "start":
            next_state = {
                **dict(auth_state),
                "pending_user_id": user_id,
                "pending_chat_id": str(chat_id),
                "pending_chat_title": chat_title,
                "pending_started_at": self._now_iso() if pending_user_id != user_id else auth_state.get("pending_started_at"),
                "pending_prompted_at": self._now_iso(),
            }
            self._state_save_bootstrap_auth(_compact_mapping(next_state))
            self._telegram_api.send_message(chat_id, self._prompt_text())
            return True
        if pending_user_id is None:
            return True
        if command_name is not None or attachments_present or not message_text:
            self._telegram_api.send_message(chat_id, self._plain_text_required_text())
            return True
        expected_password = str(self._config.repo_name or "").strip()
        if message_text == expected_password:
            failed_attempts = _telegram_bootstrap_failed_attempts(auth_state)
            failed_attempts.pop(user_id, None)
            next_state = {
                "owner_user_id": user_id,
                "owner_username": str(from_user.get("username") or "").strip() or None,
                "owner_display_name": self._user_display_name(from_user),
                "owner_chat_id": str(chat_id),
                "owner_chat_title": chat_title,
                "owner_chat_type": str(chat.get("type") or "").strip() or None,
                "owner_claimed_at": self._now_iso(),
                "failed_attempts": failed_attempts,
                "blacklist": blacklist,
            }
            self._state_save_bootstrap_auth(_compact_mapping(next_state))
            self._telegram_api.send_message(chat_id, self._success_text())
            return True
        failed_attempts = _telegram_bootstrap_failed_attempts(auth_state)
        failures = failed_attempts.get(user_id, 0) + 1
        if failures >= 3:
            blacklist[user_id] = _compact_mapping(
                {
                    "attempt_count": failures,
                    "blacklisted_at": self._now_iso(),
                    "chat_id": str(chat_id),
                    "chat_title": chat_title,
                    "username": str(from_user.get("username") or "").strip() or None,
                    "display_name": self._user_display_name(from_user),
                }
            )
            failed_attempts.pop(user_id, None)
            next_state = {
                "failed_attempts": failed_attempts,
                "blacklist": blacklist,
            }
            self._state_save_bootstrap_auth(_compact_mapping(next_state))
            self._telegram_api.send_message(chat_id, self._locked_text())
            return True
        failed_attempts[user_id] = failures
        next_state = {
            **dict(auth_state),
            "pending_user_id": user_id,
            "pending_chat_id": str(chat_id),
            "pending_chat_title": chat_title,
            "pending_started_at": auth_state.get("pending_started_at") or self._now_iso(),
            "pending_prompted_at": self._now_iso(),
            "failed_attempts": failed_attempts,
            "blacklist": blacklist,
        }
        self._state_save_bootstrap_auth(_compact_mapping(next_state))
        self._telegram_api.send_message(chat_id, self._failure_text(3 - failures))
        return True

    def _prompt_text(self) -> str:
        return "Telegram bootstrap is locked. Send the repository-name password as plain text."

    def _plain_text_required_text(self) -> str:
        return "Send the bootstrap password as plain text."

    def _success_text(self) -> str:
        return "Owner verified. Telegram access is now bound to this user id. Send /help or a normal message to continue."

    def _failure_text(self, remaining_attempts: int) -> str:
        if remaining_attempts <= 1:
            return "Incorrect password. 1 attempt remaining."
        return f"Incorrect password. {remaining_attempts} attempts remaining."

    def _locked_text(self) -> str:
        return "Incorrect password. This Telegram user id is now blocked until local reset clears the runtime auth state."

    def _existing_private_link_can_adopt_owner(
        self,
        chat_id: str | int,
        chat: Mapping[str, Any],
    ) -> bool:
        if str(chat.get("type") or "").strip() != "private":
            return False
        link = self._state_get_chat(chat_id)
        if not isinstance(link, Mapping):
            return False
        if not str(link.get("session_id") or "").strip():
            return False
        linked_chat_type = str(link.get("chat_type") or chat.get("type") or "").strip()
        if linked_chat_type and linked_chat_type != "private":
            return False
        binding_role = str(link.get("binding_role") or "").strip()
        if binding_role and binding_role != "primary_shared":
            return False
        return True

    def _adopt_owner_from_existing_private_link(
        self,
        chat_id: str | int,
        chat: Mapping[str, Any],
        from_user: Mapping[str, Any],
        chat_title: str,
        auth_state: Mapping[str, Any] | None,
    ) -> bool:
        if not self._existing_private_link_can_adopt_owner(chat_id, chat):
            return False
        user_id = str(from_user.get("id") or "").strip()
        if not user_id:
            return False
        next_state = {
            "owner_user_id": user_id,
            "owner_username": str(from_user.get("username") or "").strip() or None,
            "owner_display_name": self._user_display_name(from_user),
            "owner_chat_id": str(chat_id),
            "owner_chat_title": chat_title,
            "owner_chat_type": str(chat.get("type") or "").strip() or None,
            "owner_claimed_at": self._now_iso(),
            "owner_claim_reason": "existing_private_chat_link",
            "failed_attempts": _telegram_bootstrap_failed_attempts(auth_state),
            "blacklist": _telegram_bootstrap_blacklist(auth_state),
        }
        self._state_save_bootstrap_auth(_compact_mapping(next_state))
        return True
