from __future__ import annotations

import json
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from tempfile import NamedTemporaryFile
from typing import Any, Mapping

from ait_protocol.runtime_roots import resolve_server_runtime_root


DEFAULT_RUNTIME_BINDING_STATE_FILENAME = "telegram-sync.json"
DEFAULT_RUNTIME_BINDING_STATE_VERSION = 2
PRIMARY_SHARED_BINDING_ROLE = "primary_shared"
BRANCH_BINDING_ROLE = "branch"
NOTIFICATION_ONLY_BINDING_ROLE = "notification_only"
VALID_BINDING_ROLES = frozenset(
    {
        PRIMARY_SHARED_BINDING_ROLE,
        BRANCH_BINDING_ROLE,
        NOTIFICATION_ONLY_BINDING_ROLE,
    }
)
_UNSET = object()


def utc_now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def resolve_runtime_binding_state_path(value: str | Path | None = None) -> Path:
    if value:
        return Path(value).expanduser()
    try:
        runtime_root = resolve_server_runtime_root()
    except Exception:
        return Path.cwd() / DEFAULT_RUNTIME_BINDING_STATE_FILENAME
    return runtime_root / DEFAULT_RUNTIME_BINDING_STATE_FILENAME


def _clean_optional_str(value: object) -> str | None:
    if value is None:
        return None
    text = str(value).strip()
    return text or None


def _normalize_non_negative_int(value: object) -> int | None:
    try:
        parsed = int(value)
    except (TypeError, ValueError):
        return None
    return max(parsed, 0)


def _normalize_non_negative_float(value: object) -> float | None:
    try:
        parsed = float(value)
    except (TypeError, ValueError):
        return None
    return max(parsed, 0.0)


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


def normalize_binding_role(value: object, *, fallback: str = PRIMARY_SHARED_BINDING_ROLE) -> str:
    role = (_clean_optional_str(value) or fallback).lower()
    return role if role in VALID_BINDING_ROLES else fallback


def surface_binding_id(
    transport: str | None,
    surface_id: str | int | None,
    *,
    thread_id: str | int | None = None,
) -> str:
    transport_key = _clean_optional_str(transport) or "unknown"
    surface_key = _clean_optional_str(surface_id) or "unknown"
    thread_key = _clean_optional_str(thread_id)
    if thread_key is None:
        return f"{transport_key}:{surface_key}"
    return f"{transport_key}:{surface_key}:thread:{thread_key}"


def binding_transport(binding: Mapping[str, Any] | None) -> str | None:
    return _clean_optional_str((binding or {}).get("transport") or (binding or {}).get("surface"))


def binding_surface_id(binding: Mapping[str, Any] | None) -> str | None:
    return _clean_optional_str(
        (binding or {}).get("surface_id")
        or (binding or {}).get("transport_channel_id")
        or (binding or {}).get("chat_id")
    )


def binding_thread_id(binding: Mapping[str, Any] | None) -> str | None:
    return _clean_optional_str(
        (binding or {}).get("thread_id")
        or (binding or {}).get("transport_thread_id")
    )


def binding_canonical_session_id(binding: Mapping[str, Any] | None) -> str | None:
    return _clean_optional_str(
        (binding or {}).get("canonical_session_id")
        or (binding or {}).get("shared_session_canonical_session_id")
        or (binding or {}).get("session_id")
    )


def binding_branch_session_id(binding: Mapping[str, Any] | None) -> str | None:
    return _clean_optional_str(
        (binding or {}).get("branch_session_id")
        or (binding or {}).get("shared_session_branch_session_id")
    )


def binding_active_session_id(binding: Mapping[str, Any] | None) -> str | None:
    branch_session_id = binding_branch_session_id(binding)
    if branch_session_id is not None:
        return branch_session_id
    return _clean_optional_str(
        (binding or {}).get("session_id")
        or (binding or {}).get("active_session_id")
        or binding_canonical_session_id(binding)
    )


def binding_session_ids(binding: Mapping[str, Any] | None) -> set[str]:
    values = {
        binding_active_session_id(binding),
        binding_canonical_session_id(binding),
        binding_branch_session_id(binding),
    }
    return {value for value in values if value}


def binding_surface_label(binding: Mapping[str, Any] | None) -> str | None:
    return _clean_optional_str(
        (binding or {}).get("surface_title")
        or (binding or {}).get("chat_title")
        or binding_surface_id(binding)
    )


def default_runtime_binding_state_payload() -> dict[str, Any]:
    return {
        "version": DEFAULT_RUNTIME_BINDING_STATE_VERSION,
        "last_update_id": 0,
        "chats": {},
        "surface_bindings": {},
        "telegram_bootstrap_auth": {},
    }


@dataclass(frozen=True)
class RuntimeSurfaceBindingState:
    version: int
    last_update_id: int
    chats: dict[str, dict[str, Any]]
    surface_bindings: dict[str, dict[str, Any]]
    telegram_bootstrap_auth: dict[str, Any]


def _binding_from_legacy_chat(chat_id: str, payload: Mapping[str, Any]) -> dict[str, Any]:
    active_session_id = _clean_optional_str(payload.get("session_id"))
    canonical_session_id = _clean_optional_str(payload.get("canonical_session_id")) or active_session_id
    branch_session_id = _clean_optional_str(payload.get("branch_session_id"))
    role = normalize_binding_role(
        payload.get("binding_role"),
        fallback=BRANCH_BINDING_ROLE if branch_session_id else PRIMARY_SHARED_BINDING_ROLE,
    )
    return _compact_mapping(
        {
            "binding_id": surface_binding_id("telegram", chat_id),
            "transport": "telegram",
            "surface_id": chat_id,
            "surface_title": _clean_optional_str(payload.get("chat_title")),
            "surface_kind": _clean_optional_str(payload.get("chat_type")),
            "repo_name": _clean_optional_str(payload.get("repo_name")),
            "status": _clean_optional_str(payload.get("status")) or "active",
            "binding_role": role,
            "session_id": active_session_id,
            "canonical_session_id": canonical_session_id,
            "branch_session_id": branch_session_id,
            "previous_session_id": _clean_optional_str(payload.get("previous_session_id")),
            "branch_kind": _clean_optional_str(payload.get("branch_kind")),
            "relink_reason": _clean_optional_str(payload.get("relink_reason")),
            "linked_at": _clean_optional_str(payload.get("linked_at")),
            "updated_at": _clean_optional_str(payload.get("updated_at")),
            "relinked_at": _clean_optional_str(payload.get("relinked_at")),
            "last_synced_sequence": _normalize_non_negative_int(payload.get("last_synced_sequence")) or 0,
            "last_sync_at": _clean_optional_str(payload.get("last_sync_at")),
            "workflow_notifications_enabled": bool(payload.get("workflow_notifications_enabled", False)),
            "last_queue_summary_digest": _clean_optional_str(payload.get("last_queue_summary_digest")),
            "last_queue_notification_at": _clean_optional_str(payload.get("last_queue_notification_at")),
            "graph_watches": dict(payload.get("graph_watches") or {}) if isinstance(payload.get("graph_watches"), Mapping) else {},
            "planning_mode": payload.get("planning_mode"),
            "planning_bootstrap_sequence": _normalize_non_negative_int(payload.get("planning_bootstrap_sequence")),
            "planning_governance_context_paths": list(payload.get("planning_governance_context_paths") or []),
            "telegram_live_delivered_sequences": list(payload.get("telegram_live_delivered_sequences") or []),
            "telegram_reply_spool": list(payload.get("telegram_reply_spool") or []),
            "last_relink_skipped_reply_sequence": _normalize_non_negative_int(payload.get("last_relink_skipped_reply_sequence")),
            "last_relink_skipped_reply_at": _clean_optional_str(payload.get("last_relink_skipped_reply_at")),
            "last_relink_skipped_from_session_id": _clean_optional_str(payload.get("last_relink_skipped_from_session_id")),
            "background_sync_failure_streak": _normalize_non_negative_int(payload.get("background_sync_failure_streak")),
            "background_sync_retry_after_epoch": _normalize_non_negative_float(payload.get("background_sync_retry_after_epoch")),
            "background_sync_last_failure_at": _clean_optional_str(payload.get("background_sync_last_failure_at")),
            "background_sync_last_error": _clean_optional_str(payload.get("background_sync_last_error")),
            "runtime_backend_mode": _clean_optional_str(payload.get("runtime_backend_mode")),
            "runtime_backend_remote_name": _clean_optional_str(payload.get("runtime_backend_remote_name")),
            "runtime_backend_server_url": _clean_optional_str(payload.get("runtime_backend_server_url")),
            "runtime_backend_signature": _clean_optional_str(payload.get("runtime_backend_signature")),
        }
    )


def _legacy_chat_from_binding(binding: Mapping[str, Any], current: Mapping[str, Any] | None = None) -> dict[str, Any]:
    payload = dict(current or {})
    compact = _compact_mapping(
        {
            "chat_id": binding_surface_id(binding),
            "session_id": binding_active_session_id(binding),
            "canonical_session_id": binding_canonical_session_id(binding),
            "branch_session_id": binding_branch_session_id(binding),
            "binding_role": normalize_binding_role((binding or {}).get("binding_role")),
            "repo_name": _clean_optional_str((binding or {}).get("repo_name")),
            "chat_type": _clean_optional_str((binding or {}).get("surface_kind")),
            "chat_title": _clean_optional_str((binding or {}).get("surface_title")),
            "linked_at": _clean_optional_str((binding or {}).get("linked_at")),
            "updated_at": _clean_optional_str((binding or {}).get("updated_at")),
            "relinked_at": _clean_optional_str((binding or {}).get("relinked_at")),
            "previous_session_id": _clean_optional_str((binding or {}).get("previous_session_id")),
            "branch_kind": _clean_optional_str((binding or {}).get("branch_kind")),
            "relink_reason": _clean_optional_str((binding or {}).get("relink_reason")),
            "last_synced_sequence": _normalize_non_negative_int((binding or {}).get("last_synced_sequence")) or 0,
            "last_sync_at": _clean_optional_str((binding or {}).get("last_sync_at")),
            "workflow_notifications_enabled": bool((binding or {}).get("workflow_notifications_enabled", False)),
            "last_queue_summary_digest": _clean_optional_str((binding or {}).get("last_queue_summary_digest")),
            "last_queue_notification_at": _clean_optional_str((binding or {}).get("last_queue_notification_at")),
            "planning_mode": (binding or {}).get("planning_mode"),
            "planning_bootstrap_sequence": _normalize_non_negative_int((binding or {}).get("planning_bootstrap_sequence")),
            "planning_governance_context_paths": list((binding or {}).get("planning_governance_context_paths") or []),
            "telegram_live_delivered_sequences": list((binding or {}).get("telegram_live_delivered_sequences") or []),
            "telegram_reply_spool": list((binding or {}).get("telegram_reply_spool") or []),
            "last_relink_skipped_reply_sequence": _normalize_non_negative_int((binding or {}).get("last_relink_skipped_reply_sequence")),
            "last_relink_skipped_reply_at": _clean_optional_str((binding or {}).get("last_relink_skipped_reply_at")),
            "last_relink_skipped_from_session_id": _clean_optional_str((binding or {}).get("last_relink_skipped_from_session_id")),
            "background_sync_failure_streak": _normalize_non_negative_int((binding or {}).get("background_sync_failure_streak")),
            "background_sync_retry_after_epoch": _normalize_non_negative_float((binding or {}).get("background_sync_retry_after_epoch")),
            "background_sync_last_failure_at": _clean_optional_str((binding or {}).get("background_sync_last_failure_at")),
            "background_sync_last_error": _clean_optional_str((binding or {}).get("background_sync_last_error")),
            "runtime_backend_mode": _clean_optional_str((binding or {}).get("runtime_backend_mode")),
            "runtime_backend_remote_name": _clean_optional_str((binding or {}).get("runtime_backend_remote_name")),
            "runtime_backend_server_url": _clean_optional_str((binding or {}).get("runtime_backend_server_url")),
            "runtime_backend_signature": _clean_optional_str((binding or {}).get("runtime_backend_signature")),
        }
        )
    if "graph_watches" in binding:
        compact["graph_watches"] = (
            dict((binding or {}).get("graph_watches") or {})
            if isinstance((binding or {}).get("graph_watches"), Mapping)
            else {}
        )
    payload.update(compact)
    for clearable_key, raw_value in {
        "telegram_live_delivered_sequences": list((binding or {}).get("telegram_live_delivered_sequences") or []),
        "telegram_reply_spool": list((binding or {}).get("telegram_reply_spool") or []),
        "background_sync_failure_streak": _normalize_non_negative_int((binding or {}).get("background_sync_failure_streak")),
        "background_sync_retry_after_epoch": _normalize_non_negative_float((binding or {}).get("background_sync_retry_after_epoch")),
        "background_sync_last_failure_at": _clean_optional_str((binding or {}).get("background_sync_last_failure_at")),
        "background_sync_last_error": _clean_optional_str((binding or {}).get("background_sync_last_error")),
        "last_relink_skipped_reply_sequence": _normalize_non_negative_int((binding or {}).get("last_relink_skipped_reply_sequence")),
        "last_relink_skipped_reply_at": _clean_optional_str((binding or {}).get("last_relink_skipped_reply_at")),
        "last_relink_skipped_from_session_id": _clean_optional_str((binding or {}).get("last_relink_skipped_from_session_id")),
    }.items():
        if raw_value in (None, [], {}):
            payload.pop(clearable_key, None)
        else:
            payload[clearable_key] = raw_value
    payload["chat_id"] = binding_surface_id(binding) or ""
    payload["session_id"] = binding_active_session_id(binding) or ""
    payload["canonical_session_id"] = binding_canonical_session_id(binding) or ""
    payload["branch_session_id"] = binding_branch_session_id(binding) or ""
    payload["repo_name"] = _clean_optional_str((binding or {}).get("repo_name")) or ""
    return payload


def load_runtime_binding_state(path: Path | None = None) -> RuntimeSurfaceBindingState:
    target = path or resolve_runtime_binding_state_path()
    if not target.exists():
        payload = default_runtime_binding_state_payload()
    else:
        payload = json.loads(target.read_text(encoding="utf-8"))
        if not isinstance(payload, dict):
            payload = default_runtime_binding_state_payload()
    chats = payload.get("chats")
    if not isinstance(chats, dict):
        chats = {}
    raw_bindings = payload.get("surface_bindings")
    if not isinstance(raw_bindings, dict):
        raw_bindings = {}
    raw_bootstrap_auth = payload.get("telegram_bootstrap_auth")
    if not isinstance(raw_bootstrap_auth, dict):
        raw_bootstrap_auth = {}
    normalized_chats = {str(key): dict(value) for key, value in chats.items() if isinstance(value, Mapping)}
    normalized_bindings = {str(key): dict(value) for key, value in raw_bindings.items() if isinstance(value, Mapping)}
    for chat_id, link in normalized_chats.items():
        binding = _binding_from_legacy_chat(chat_id, link)
        binding_id = str(binding.get("binding_id") or surface_binding_id("telegram", chat_id))
        normalized_bindings.setdefault(binding_id, binding)
    return RuntimeSurfaceBindingState(
        version=int(payload.get("version") or DEFAULT_RUNTIME_BINDING_STATE_VERSION),
        last_update_id=int(payload.get("last_update_id") or 0),
        chats=normalized_chats,
        surface_bindings=normalized_bindings,
        telegram_bootstrap_auth={str(key): value for key, value in raw_bootstrap_auth.items()},
    )


def save_runtime_binding_state(state: RuntimeSurfaceBindingState | Mapping[str, Any], path: Path | None = None) -> None:
    target = path or resolve_runtime_binding_state_path()
    target.parent.mkdir(parents=True, exist_ok=True)
    payload = state if isinstance(state, Mapping) else {
        "version": int(state.version or DEFAULT_RUNTIME_BINDING_STATE_VERSION),
        "last_update_id": int(state.last_update_id or 0),
        "chats": state.chats,
        "surface_bindings": state.surface_bindings,
        "telegram_bootstrap_auth": state.telegram_bootstrap_auth,
    }
    with NamedTemporaryFile("w", encoding="utf-8", dir=target.parent, delete=False) as handle:
        json.dump(payload, handle, ensure_ascii=False, indent=2, sort_keys=True)
        handle.write("\n")
        tmp_path = Path(handle.name)
    tmp_path.replace(target)


class RuntimeSurfaceBindingStore:
    def __init__(self, path: Path | None = None):
        self.path = path or resolve_runtime_binding_state_path()

    def load(self) -> RuntimeSurfaceBindingState:
        return load_runtime_binding_state(self.path)

    def save(self, state: RuntimeSurfaceBindingState | Mapping[str, Any]) -> None:
        save_runtime_binding_state(state, self.path)

    def update_last_update_id(self, update_id: int) -> RuntimeSurfaceBindingState:
        state = self.load()
        if int(update_id) <= state.last_update_id:
            return state
        next_state = RuntimeSurfaceBindingState(
            version=state.version,
            last_update_id=int(update_id),
            chats=state.chats,
            surface_bindings=state.surface_bindings,
            telegram_bootstrap_auth=state.telegram_bootstrap_auth,
        )
        self.save(next_state)
        return next_state

    def _binding_lookup_key(
        self,
        *,
        binding_id: str | None = None,
        transport: str | None = None,
        surface_id: str | int | None = None,
        thread_id: str | int | None = None,
    ) -> str:
        if binding_id:
            return str(binding_id)
        return surface_binding_id(transport, surface_id, thread_id=thread_id)

    def get_binding(
        self,
        *,
        binding_id: str | None = None,
        transport: str | None = None,
        surface_id: str | int | None = None,
        thread_id: str | int | None = None,
    ) -> dict[str, Any] | None:
        state = self.load()
        key = self._binding_lookup_key(
            binding_id=binding_id,
            transport=transport,
            surface_id=surface_id,
            thread_id=thread_id,
        )
        binding = state.surface_bindings.get(key)
        return dict(binding) if isinstance(binding, Mapping) else None

    def list_bindings(
        self,
        *,
        repo_name: str | None = None,
        transport: str | None = None,
        include_inactive: bool = False,
    ) -> list[dict[str, Any]]:
        state = self.load()
        bindings: list[dict[str, Any]] = []
        for row in state.surface_bindings.values():
            if not isinstance(row, Mapping):
                continue
            candidate = dict(row)
            if repo_name is not None and _clean_optional_str(candidate.get("repo_name")) != _clean_optional_str(repo_name):
                continue
            if transport is not None and binding_transport(candidate) != _clean_optional_str(transport):
                continue
            if not include_inactive and (_clean_optional_str(candidate.get("status")) or "active") != "active":
                continue
            bindings.append(candidate)
        return bindings

    def _upsert_binding_locked(
        self,
        state: RuntimeSurfaceBindingState,
        *,
        transport: str,
        surface_id: str | int,
        repo_name: str,
        surface_title: str | None = None,
        surface_kind: str | None = None,
        thread_id: str | int | None | object = _UNSET,
        session_id: str | None | object = _UNSET,
        canonical_session_id: str | None | object = _UNSET,
        branch_session_id: str | None | object = _UNSET,
        binding_role: str | None = None,
        status: str | None = None,
        last_synced_sequence: int | None = None,
        last_sync_at: str | None = None,
        updates: Mapping[str, Any] | None = None,
    ) -> tuple[RuntimeSurfaceBindingState, dict[str, Any]]:
        thread_value = (
            None if thread_id is _UNSET else thread_id
        )
        key = surface_binding_id(transport, surface_id, thread_id=thread_value)
        current = dict(state.surface_bindings.get(key) or {})
        now = utc_now_iso()
        if current:
            linked_at = _clean_optional_str(current.get("linked_at")) or now
        else:
            linked_at = now
        existing_active = binding_active_session_id(current)
        existing_canonical = binding_canonical_session_id(current)
        existing_branch = binding_branch_session_id(current)
        next_branch = existing_branch if branch_session_id is _UNSET else _clean_optional_str(branch_session_id)
        next_canonical = existing_canonical if canonical_session_id is _UNSET else _clean_optional_str(canonical_session_id)
        next_session = existing_active if session_id is _UNSET else _clean_optional_str(session_id)
        if next_branch and next_canonical is None:
            next_canonical = existing_canonical or existing_active
        if next_session is None:
            next_session = next_branch or next_canonical
        if next_canonical is None:
            next_canonical = next_session
        if next_branch and next_session != next_branch:
            next_session = next_branch
        role_fallback = BRANCH_BINDING_ROLE if next_branch else PRIMARY_SHARED_BINDING_ROLE
        payload = dict(current)
        payload.update(
            {
                "binding_id": key,
                "transport": _clean_optional_str(transport) or "unknown",
                "surface_id": _clean_optional_str(surface_id) or "",
                "surface_title": _clean_optional_str(surface_title) or _clean_optional_str(current.get("surface_title")),
                "surface_kind": _clean_optional_str(surface_kind) or _clean_optional_str(current.get("surface_kind")),
                "thread_id": _clean_optional_str(thread_value) if thread_id is not _UNSET else _clean_optional_str(current.get("thread_id")),
                "repo_name": _clean_optional_str(repo_name) or _clean_optional_str(current.get("repo_name")) or "",
                "status": _clean_optional_str(status) or _clean_optional_str(current.get("status")) or "active",
                "binding_role": normalize_binding_role(binding_role or current.get("binding_role"), fallback=role_fallback),
                "session_id": next_session,
                "canonical_session_id": next_canonical,
                "branch_session_id": next_branch,
                "linked_at": linked_at,
                "updated_at": now,
            }
        )
        if last_synced_sequence is not None:
            payload["last_synced_sequence"] = max(int(last_synced_sequence), 0)
        elif "last_synced_sequence" not in payload:
            payload["last_synced_sequence"] = 0
        if last_sync_at is not None:
            payload["last_sync_at"] = last_sync_at
        explicit_graph_watches: dict[str, Any] | None = None
        if updates:
            payload.update(dict(updates))
            if "graph_watches" in updates:
                explicit_graph_watches = (
                    dict(updates.get("graph_watches") or {})
                    if isinstance(updates.get("graph_watches"), Mapping)
                    else {}
                )
        payload = _compact_mapping(payload)
        if explicit_graph_watches is not None:
            payload["graph_watches"] = explicit_graph_watches
        next_bindings = dict(state.surface_bindings)
        next_bindings[key] = payload
        next_chats = dict(state.chats)
        if binding_transport(payload) == "telegram" and binding_surface_id(payload) is not None:
            next_chats[str(binding_surface_id(payload))] = _legacy_chat_from_binding(
                payload,
                current=next_chats.get(str(binding_surface_id(payload))) if isinstance(next_chats.get(str(binding_surface_id(payload))), Mapping) else None,
            )
        next_state = RuntimeSurfaceBindingState(
            version=max(int(state.version or 0), DEFAULT_RUNTIME_BINDING_STATE_VERSION),
            last_update_id=state.last_update_id,
            chats=next_chats,
            surface_bindings=next_bindings,
            telegram_bootstrap_auth=state.telegram_bootstrap_auth,
        )
        return next_state, payload

    def upsert_binding(
        self,
        *,
        transport: str,
        surface_id: str | int,
        repo_name: str,
        surface_title: str | None = None,
        surface_kind: str | None = None,
        thread_id: str | int | None | object = _UNSET,
        session_id: str | None | object = _UNSET,
        canonical_session_id: str | None | object = _UNSET,
        branch_session_id: str | None | object = _UNSET,
        binding_role: str | None = None,
        status: str | None = None,
        last_synced_sequence: int | None = None,
        last_sync_at: str | None = None,
        updates: Mapping[str, Any] | None = None,
    ) -> dict[str, Any]:
        state = self.load()
        next_state, payload = self._upsert_binding_locked(
            state,
            transport=transport,
            surface_id=surface_id,
            repo_name=repo_name,
            surface_title=surface_title,
            surface_kind=surface_kind,
            thread_id=thread_id,
            session_id=session_id,
            canonical_session_id=canonical_session_id,
            branch_session_id=branch_session_id,
            binding_role=binding_role,
            status=status,
            last_synced_sequence=last_synced_sequence,
            last_sync_at=last_sync_at,
            updates=updates,
        )
        self.save(next_state)
        return payload

    def patch_binding(
        self,
        *,
        binding_id: str | None = None,
        transport: str | None = None,
        surface_id: str | int | None = None,
        thread_id: str | int | None = None,
        **updates: Any,
    ) -> dict[str, Any] | None:
        state = self.load()
        key = self._binding_lookup_key(
            binding_id=binding_id,
            transport=transport,
            surface_id=surface_id,
            thread_id=thread_id,
        )
        current = state.surface_bindings.get(key)
        if not isinstance(current, Mapping):
            return None
        known = dict(updates)
        next_state, payload = self._upsert_binding_locked(
            state,
            transport=binding_transport(current) or "unknown",
            surface_id=binding_surface_id(current) or "",
            repo_name=_clean_optional_str(current.get("repo_name")) or "",
            surface_title=_clean_optional_str(known.pop("surface_title", known.pop("chat_title", current.get("surface_title")))),
            surface_kind=_clean_optional_str(known.pop("surface_kind", known.pop("chat_type", current.get("surface_kind")))),
            thread_id=known.pop("thread_id", binding_thread_id(current)),
            session_id=known.pop("session_id", _UNSET),
            canonical_session_id=known.pop("canonical_session_id", _UNSET),
            branch_session_id=known.pop("branch_session_id", _UNSET),
            binding_role=known.pop("binding_role", None),
            status=known.pop("status", None),
            last_synced_sequence=known.pop("last_synced_sequence", None),
            last_sync_at=known.pop("last_sync_at", None),
            updates=known,
        )
        self.save(next_state)
        return payload

    def linkage_by_session(self, *, include_branch_aliases: bool = False) -> dict[str, dict[str, Any]]:
        state = self.load()
        links: dict[str, dict[str, Any]] = {}
        for binding in state.surface_bindings.values():
            if not isinstance(binding, Mapping):
                continue
            active_session_id = binding_active_session_id(binding)
            if active_session_id:
                links[active_session_id] = dict(binding)
            if include_branch_aliases:
                canonical_session_id = binding_canonical_session_id(binding)
                if canonical_session_id and canonical_session_id not in links:
                    links[canonical_session_id] = dict(binding)
        return links

    def bindings_for_session(self, session_id: str, *, include_inactive: bool = False) -> list[dict[str, Any]]:
        wanted = _clean_optional_str(session_id)
        if wanted is None:
            return []
        matches: list[dict[str, Any]] = []
        for binding in self.list_bindings(include_inactive=include_inactive):
            if wanted in binding_session_ids(binding):
                matches.append(binding)
        return matches

    def resolve_repo_shared_binding(self, repo_name: str) -> dict[str, Any]:
        normalized_repo = _clean_optional_str(repo_name)
        if normalized_repo is None:
            return {
                "status": "missing",
                "detail": "Repository selection is required before resolving a shared session binding.",
                "bindings": [],
            }
        state = self.load()
        bindings = [
            dict(binding)
            for binding in state.surface_bindings.values()
            if isinstance(binding, Mapping)
            and _clean_optional_str(binding.get("repo_name")) == normalized_repo
            and (_clean_optional_str(binding.get("status")) or "active") == "active"
            and binding_canonical_session_id(binding)
        ]
        if not bindings:
            return {
                "status": "missing",
                "detail": "No shared session binding is currently available for this repository.",
                "bindings": [],
            }
        canonical_ids = sorted({binding_canonical_session_id(binding) for binding in bindings if binding_canonical_session_id(binding)})
        if len(canonical_ids) > 1:
            return {
                "status": "ambiguous",
                "detail": "Multiple canonical shared sessions are linked for this repository; resolve the primary binding before using the shared web chat.",
                "bindings": bindings,
                "canonical_session_ids": canonical_ids,
            }
        bindings.sort(
            key=lambda row: (
                normalize_binding_role(row.get("binding_role"), fallback=PRIMARY_SHARED_BINDING_ROLE) == BRANCH_BINDING_ROLE,
                _clean_optional_str(row.get("updated_at")) or "",
            ),
            reverse=True,
        )
        binding = bindings[0]
        binding_id = str(binding.get("binding_id") or "")
        resolution_mode = "surface_binding" if binding_id in state.surface_bindings else "telegram_compat_fallback"
        return {
            "status": "resolved",
            "detail": "Resolved the shared session from the runtime surface binding store.",
            "binding": binding,
            "bindings": bindings,
            "canonical_session_id": canonical_ids[0],
            "resolution_mode": resolution_mode,
        }


__all__ = [
    "BRANCH_BINDING_ROLE",
    "DEFAULT_RUNTIME_BINDING_STATE_FILENAME",
    "DEFAULT_RUNTIME_BINDING_STATE_VERSION",
    "NOTIFICATION_ONLY_BINDING_ROLE",
    "PRIMARY_SHARED_BINDING_ROLE",
    "RuntimeSurfaceBindingState",
    "RuntimeSurfaceBindingStore",
    "binding_active_session_id",
    "binding_branch_session_id",
    "binding_canonical_session_id",
    "binding_session_ids",
    "binding_surface_id",
    "binding_surface_label",
    "binding_thread_id",
    "binding_transport",
    "default_runtime_binding_state_payload",
    "load_runtime_binding_state",
    "normalize_binding_role",
    "resolve_runtime_binding_state_path",
    "save_runtime_binding_state",
    "surface_binding_id",
    "utc_now_iso",
]
