from __future__ import annotations

from typing import Any, Callable, Mapping


def _clean_optional_str(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _runtime_link_fields(runtime_snapshot: Any | None) -> dict[str, Any]:
    if runtime_snapshot is None:
        return {}
    return {
        "runtime_backend_mode": runtime_snapshot.mode,
        "runtime_backend_remote_name": runtime_snapshot.remote_name,
        "runtime_backend_server_url": runtime_snapshot.server_url,
        "runtime_backend_signature": runtime_snapshot.signature,
    }


def _runtime_signature_from_link(link: Mapping[str, Any] | None) -> str | None:
    if not isinstance(link, Mapping):
        return None
    return _clean_optional_str(link.get("runtime_backend_signature"))


class TelegramSessionLinkCoordinator:
    def __init__(
        self,
        *,
        config: Any,
        ait_api_call: Callable[..., Any],
        state_get_chat: Callable[[str | int], dict[str, Any] | None],
        state_upsert_chat: Callable[..., dict[str, Any]],
        state_patch_chat: Callable[..., dict[str, Any] | None],
        runtime_snapshot: Callable[[], Any | None],
        now_iso: Callable[[], str],
        runtime_error_type: type[Exception],
    ) -> None:
        self._config = config
        self._ait_api_call = ait_api_call
        self._state_get_chat = state_get_chat
        self._state_upsert_chat = state_upsert_chat
        self._state_patch_chat = state_patch_chat
        self._runtime_snapshot = runtime_snapshot
        self._now_iso = now_iso
        self._runtime_error_type = runtime_error_type

    def session_missing_relink_reason(
        self,
        link: Mapping[str, Any] | None,
        runtime_snapshot: Any | None,
        *,
        startup_signature: str,
    ) -> str:
        if runtime_snapshot is None:
            return "session_missing"
        link_signature = _runtime_signature_from_link(link)
        if link_signature and link_signature != runtime_snapshot.signature:
            return "runtime_backend_changed"
        if runtime_snapshot.signature != startup_signature:
            return "runtime_backend_changed"
        return "session_missing"

    def create_transport_session(
        self,
        *,
        runtime_snapshot: Any | None = None,
        **kwargs: Any,
    ) -> dict[str, Any]:
        try:
            return self._ait_api_call("create_session", runtime_snapshot=runtime_snapshot, **kwargs)
        except TypeError as exc:
            if "unexpected keyword argument" not in str(exc):
                raise
        fallback_kwargs = {
            "chat_id": kwargs["chat_id"],
            "chat_title": kwargs.get("chat_title"),
            "chat_type": kwargs.get("chat_type"),
            "session_kind": kwargs.get("session_kind", "telegram_chat"),
            "title_prefix": kwargs.get("title_prefix", "Telegram chat"),
            "metadata_extra": kwargs.get("metadata_extra"),
        }
        return self._ait_api_call("create_session", runtime_snapshot=runtime_snapshot, **fallback_kwargs)

    def create_fresh_session(
        self,
        chat_id: str | int,
        chat: dict[str, Any],
        chat_title: str,
        *,
        runtime_snapshot: Any | None = None,
        previous_link: Mapping[str, Any] | None = None,
        relink_reason: str = "initial_link",
    ) -> dict[str, Any]:
        previous_session_id = (
            _clean_optional_str((previous_link or {}).get("session_id"))
            or _clean_optional_str((previous_link or {}).get("previous_session_id"))
        )
        session = self.create_transport_session(
            runtime_snapshot=runtime_snapshot,
            chat_id=str(chat_id),
            chat_title=chat_title,
            chat_type=chat.get("type"),
            binding_role="primary_shared",
            canonical_session_id=None,
            active_session_id=previous_session_id,
            branch_session_id=None,
            branch_kind=None,
            relink_reason=relink_reason,
        )
        return self._state_upsert_chat(
            chat_id,
            session_id=session["session_id"],
            repo_name=self._config.repo_name,
            chat_type=chat.get("type"),
            chat_title=chat_title,
            canonical_session_id=session["session_id"],
            branch_session_id=None,
            binding_role="primary_shared",
            last_synced_sequence=int(session.get("last_event_sequence") or 0),
            last_sync_at=self._now_iso(),
            previous_session_id=previous_session_id,
            branch_kind=None,
            relink_reason=relink_reason,
            relinked_at=self._now_iso() if previous_session_id else None,
            **_runtime_link_fields(runtime_snapshot),
        )

    def ensure_session_link(
        self,
        chat_id: str | int,
        chat: dict[str, Any],
        chat_title: str,
        *,
        create_if_missing: bool = True,
        runtime_snapshot: Any | None = None,
        startup_signature: str,
    ) -> dict[str, Any] | None:
        link = self._state_get_chat(chat_id)
        snapshot = runtime_snapshot or self._runtime_snapshot()
        if link and link.get("session_id"):
            if snapshot is not None:
                link_signature = _runtime_signature_from_link(link)
                if link_signature and link_signature != snapshot.signature:
                    if create_if_missing:
                        return self.create_fresh_session(
                            chat_id,
                            chat,
                            chat_title,
                            runtime_snapshot=snapshot,
                            previous_link=link,
                            relink_reason="runtime_backend_changed",
                        )
                    return self._state_upsert_chat(
                        chat_id,
                        session_id=str(link["session_id"]),
                        repo_name=self._config.repo_name,
                        chat_type=chat.get("type"),
                        chat_title=chat_title,
                        canonical_session_id=str(link.get("canonical_session_id") or "").strip() or None,
                        branch_session_id=str(link.get("branch_session_id") or "").strip() or None,
                        binding_role=str(link.get("binding_role") or "").strip() or None,
                        previous_session_id=str(link.get("previous_session_id") or "").strip() or None,
                        relink_reason="runtime_backend_changed",
                        **_runtime_link_fields(snapshot),
                    )
            try:
                self._ait_api_call(
                    "get_session",
                    str(link["session_id"]),
                    runtime_snapshot=snapshot,
                )
                return self._state_upsert_chat(
                    chat_id,
                    session_id=str(link["session_id"]),
                    repo_name=self._config.repo_name,
                    chat_type=chat.get("type"),
                    chat_title=chat_title,
                    canonical_session_id=str(link.get("canonical_session_id") or "").strip() or None,
                    branch_session_id=str(link.get("branch_session_id") or "").strip() or None,
                    binding_role=str(link.get("binding_role") or "").strip() or None,
                    relink_reason=str(link.get("relink_reason") or "").strip() or None,
                    **_runtime_link_fields(snapshot),
                )
            except self._runtime_error_type:
                canonical_session_id = str(link.get("canonical_session_id") or "").strip()
                branch_session_id = str(link.get("branch_session_id") or "").strip()
                if canonical_session_id and branch_session_id and canonical_session_id != branch_session_id:
                    try:
                        self._ait_api_call(
                            "get_session",
                            canonical_session_id,
                            runtime_snapshot=snapshot,
                        )
                        return self._state_upsert_chat(
                            chat_id,
                            session_id=canonical_session_id,
                            repo_name=self._config.repo_name,
                            chat_type=chat.get("type"),
                            chat_title=chat_title,
                            canonical_session_id=canonical_session_id,
                            branch_session_id=None,
                            binding_role="primary_shared",
                            previous_session_id=branch_session_id,
                            relink_reason="branch_session_missing",
                            relinked_at=self._now_iso(),
                            **_runtime_link_fields(snapshot),
                        )
                    except self._runtime_error_type:
                        pass
        if not create_if_missing:
            if link and link.get("session_id"):
                return self._state_upsert_chat(
                    chat_id,
                    session_id=str(link["session_id"]),
                    repo_name=self._config.repo_name,
                    chat_type=chat.get("type"),
                    chat_title=chat_title,
                    canonical_session_id=str(link.get("canonical_session_id") or "").strip() or None,
                    branch_session_id=str(link.get("branch_session_id") or "").strip() or None,
                    binding_role=str(link.get("binding_role") or "").strip() or None,
                    previous_session_id=str(link.get("previous_session_id") or "").strip() or None,
                    relink_reason=self.session_missing_relink_reason(link, snapshot, startup_signature=startup_signature),
                    **_runtime_link_fields(snapshot),
                )
            return None
        return self.create_fresh_session(
            chat_id,
            chat,
            chat_title,
            runtime_snapshot=snapshot,
            previous_link=link,
            relink_reason=self.session_missing_relink_reason(link, snapshot, startup_signature=startup_signature)
            if link
            else "initial_link",
        )

    def sync_session(
        self,
        chat_id: str | int,
        link: dict[str, Any],
        *,
        should_skip_event_for_chat: Callable[[str | int, dict[str, Any]], bool],
    ) -> list[dict[str, Any]]:
        session_id = str(link["session_id"])
        after_sequence = int(link.get("last_synced_sequence") or 0)
        runtime_snapshot = self._runtime_snapshot()
        events = self._ait_api_call(
            "list_session_events",
            session_id,
            after_sequence=after_sequence,
            runtime_snapshot=runtime_snapshot,
        )
        filtered = [event for event in events if not should_skip_event_for_chat(chat_id, event)]
        max_sequence = after_sequence
        for event in events:
            max_sequence = max(max_sequence, int(event.get("sequence") or 0))
        self._state_upsert_chat(
            chat_id,
            session_id=session_id,
            repo_name=self._config.repo_name,
            chat_type=link.get("chat_type"),
            chat_title=link.get("chat_title"),
            last_synced_sequence=max_sequence,
            last_sync_at=self._now_iso(),
            **_runtime_link_fields(runtime_snapshot),
        )
        return filtered

    def mark_missing_session_relink_required(
        self,
        chat_id: str | int,
        link: Mapping[str, Any],
        *,
        runtime_snapshot: Any | None = None,
        relink_reason: str,
    ) -> dict[str, Any] | None:
        snapshot = runtime_snapshot or self._runtime_snapshot()
        active_session_id = str(link.get("session_id") or "").strip()
        canonical_session_id = str(link.get("canonical_session_id") or "").strip()
        previous_session_id = active_session_id or str(link.get("previous_session_id") or "").strip() or None
        if canonical_session_id and canonical_session_id != active_session_id:
            try:
                self._ait_api_call(
                    "get_session",
                    canonical_session_id,
                    runtime_snapshot=snapshot,
                )
            except self._runtime_error_type:
                canonical_session_id = ""
            else:
                return self._state_patch_chat(
                    chat_id,
                    session_id=canonical_session_id,
                    canonical_session_id=canonical_session_id,
                    branch_session_id=None,
                    binding_role="primary_shared",
                    previous_session_id=previous_session_id,
                    branch_kind=None,
                    relink_reason="branch_session_missing",
                    relinked_at=self._now_iso(),
                    last_sync_at=self._now_iso(),
                    **_runtime_link_fields(snapshot),
                )
        return self.create_fresh_session(
            chat_id,
            {"type": _clean_optional_str(link.get("chat_type"))},
            _clean_optional_str(link.get("chat_title")) or str(chat_id),
            runtime_snapshot=snapshot,
            previous_link={
                "session_id": previous_session_id,
                "previous_session_id": previous_session_id,
            },
            relink_reason=relink_reason,
        )
